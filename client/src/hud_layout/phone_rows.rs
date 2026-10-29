//! Renderer-free phone HUD geometry. The runtime and fast contract tests use
//! this same implementation; no Bevy, window, assets or GPU are required.

pub(crate) const ALLY_CELL: f32 = 32.0;
pub(crate) const ALLY_GAP: f32 = 2.0;
pub(crate) const ALLY_HEIGHT: f32 = 44.0;
pub(crate) const TARGET_WIDTH: f32 = 144.0;
pub(crate) const TARGET_MIN_WIDTH: f32 = 126.0;
pub(crate) const TARGET_HEIGHT: f32 = 40.0;
pub(crate) const DEATH_CELL: f32 = 32.0;
pub(crate) const GAP: f32 = 4.0;
pub(crate) const FEED_HEIGHT: f32 = 30.0;
pub(crate) const FEED_SHORT_HEIGHT: f32 = 16.0;
pub(crate) const BUY_SLOT: f32 = 44.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Box2 {
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
}
impl Box2 {
    pub const fn new(left: f32, top: f32, width: f32, height: f32) -> Self {
        Self {
            left,
            top,
            width,
            height,
        }
    }
    pub fn right(self) -> f32 {
        self.left + self.width
    }
    pub fn bottom(self) -> f32 {
        self.top + self.height
    }
    pub fn center_x(self) -> f32 {
        self.left + self.width * 0.5
    }
    pub fn overlaps(self, other: Self) -> bool {
        self.left < other.right()
            && self.right() > other.left
            && self.top < other.bottom()
            && self.bottom() > other.top
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Input {
    pub width: f32,
    pub height: f32,
    pub safe_left: f32,
    pub safe_right: f32,
    pub safe_top: f32,
    pub map_side: f32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Layout {
    pub minimap: Box2,
    pub allies: Box2,
    pub target: Box2,
    pub score: Box2,
    pub menu: Box2,
    pub chat: Box2,
    pub deaths: [Box2; 5],
    pub quick_buy: Box2,
    pub kill_feed: Box2,
    pub player_status: Box2,
    pub brush: Box2,
    pub feedback: Box2,
}

impl Layout {
    pub fn ally_cells(self) -> [Box2; 4] {
        std::array::from_fn(|i| {
            Box2::new(
                self.allies.left + i as f32 * (ALLY_CELL + ALLY_GAP),
                self.allies.top,
                ALLY_CELL,
                ALLY_HEIGHT,
            )
        })
    }
    /// Live locks retain their existing two-row right-side orbit. Deaths do
    /// not consume these indices or move a living enemy to another slot.
    pub fn live_slots(self) -> [Box2; 5] {
        std::array::from_fn(|i| {
            Box2::new(
                self.score.right() - 44.0 - (i / 2) as f32 * 48.0,
                self.score.top + 44.0 + GAP + (i % 2) as f32 * 48.0,
                44.0,
                44.0,
            )
        })
    }
}

pub(crate) fn solve(input: Input) -> Layout {
    let left = input.safe_left + 10.0;
    let right = input.width - input.safe_right - 10.0;
    let top = input.safe_top.max(4.0);
    let center = (input.safe_left + input.width - input.safe_right) * 0.5;
    let score = Box2::new(right - 160.0, top, 160.0, 24.0);
    let ally_width = ALLY_CELL * 4.0 + ALLY_GAP * 3.0;
    let ordinary_allies = Box2::new(left + input.map_side + 6.0, top, ally_width, ALLY_HEIGHT);
    let target_budget =
        |allies: Box2| 2.0 * (center - allies.right() - GAP).min(score.left - GAP - center);
    // Compression is the first response. If fixed-size map + four real ally
    // cells cannot leave a readable target, the map moves below the header;
    // neither avatars nor the centered selected target ever move down.
    let map_below_header = target_budget(ordinary_allies) < TARGET_MIN_WIDTH;
    let (allies, minimap) = if map_below_header {
        (
            Box2::new(left, top, ally_width, ALLY_HEIGHT),
            Box2::new(
                left,
                top + ALLY_HEIGHT + GAP,
                input.map_side.min(104.0),
                input.map_side.min(104.0),
            ),
        )
    } else {
        (
            ordinary_allies,
            Box2::new(left, top, input.map_side, input.map_side),
        )
    };
    let target_width = TARGET_WIDTH.min(target_budget(allies).max(0.0));
    let target = Box2::new(
        center - target_width * 0.5,
        top,
        target_width,
        TARGET_HEIGHT,
    );
    // In the narrow header fallback the map occupies the space directly
    // under the allies; keep deaths alongside it on the same lower row.
    let death_left = allies.left.max(minimap.right() + 6.0);
    let deaths = std::array::from_fn(|i| {
        Box2::new(
            death_left + i as f32 * (DEATH_CELL + ALLY_GAP),
            allies.bottom() + GAP,
            DEATH_CELL,
            DEATH_CELL,
        )
    });
    let service_top = deaths[0].bottom() + GAP;
    let menu = Box2::new(minimap.right() + GAP, service_top, 44.0, 44.0);
    let chat = Box2::new(menu.right(), service_top, 44.0, 44.0);
    let quick_buy = Box2::new(
        input.safe_left,
        minimap.bottom() + GAP,
        3.0 * BUY_SLOT,
        BUY_SLOT,
    );
    let kill_feed = Box2::new(
        left,
        quick_buy.bottom() + GAP,
        180.0_f32.min((input.width - input.safe_left - input.safe_right - 20.0).max(0.0)),
        if input.height <= 340.0 {
            FEED_SHORT_HEIGHT
        } else {
            FEED_HEIGHT
        },
    );
    let player_status = Box2::new(left, kill_feed.bottom() + GAP, 200.0, 48.0);
    let mut brush = Box2::new(
        target.center_x() - 72.0,
        (top + 52.0).max(deaths[0].bottom() + 8.0),
        144.0,
        24.0,
    );
    if brush.overlaps(menu) || brush.overlaps(chat) {
        brush.top = chat.bottom() + GAP;
    }
    let mut feedback = Box2::new(target.center_x() - 110.0, brush.bottom() + GAP, 220.0, 40.0);
    // This preserves a bounded information stack. Extremely narrow-phone
    // transient-toast/combat artwork comfort remains a separate concern.
    for occupied in [minimap, quick_buy, kill_feed, player_status, menu, chat] {
        if feedback.overlaps(occupied) {
            feedback.left = occupied.right() + GAP;
        }
    }
    let mut layout = Layout {
        minimap,
        allies,
        target,
        score,
        menu,
        chat,
        deaths,
        quick_buy,
        kill_feed,
        player_status,
        brush,
        feedback,
    };
    if layout
        .live_slots()
        .into_iter()
        .any(|slot| layout.feedback.overlaps(slot))
    {
        layout.feedback.top = layout
            .live_slots()
            .into_iter()
            .map(Box2::bottom)
            .fold(0.0, f32::max)
            + GAP;
        for occupied in [minimap, quick_buy, kill_feed, player_status, menu, chat] {
            if layout.feedback.overlaps(occupied) {
                layout.feedback.left = occupied.right() + GAP;
            }
        }
    }
    layout
}

#[cfg(test)]
mod tests {
    use super::*;
    fn phone(width: f32, height: f32, left: f32, right: f32, top: f32) -> Input {
        Input {
            width,
            height,
            safe_left: left,
            safe_right: right,
            safe_top: top,
            map_side: if width.min(height) >= 600.0 {
                144.0
            } else {
                120.0
            },
        }
    }
    fn contract(input: Input, p: Layout) -> Result<(), String> {
        let top = input.safe_top.max(4.0);
        if p.allies.top != top {
            return Err("allied portraits dropped below the top".into());
        }
        if p.target.top != top {
            return Err("selected target dropped below the top".into());
        }
        if (p.target.center_x() - (input.safe_left + input.width - input.safe_right) * 0.5).abs()
            > 0.001
        {
            return Err("selected target displaced horizontally".into());
        }
        if !(TARGET_MIN_WIDTH..=TARGET_WIDTH).contains(&p.target.width) {
            return Err(format!("target width {}", p.target.width));
        }
        let bounds = |name: &str, a: Box2| -> Result<(), String> {
            if [a.left, a.top, a.width, a.height]
                .into_iter()
                .any(|v| !v.is_finite())
                || a.width <= 0.0
                || a.height <= 0.0
            {
                return Err(format!("non-positive/non-finite {name}: {a:?}"));
            }
            if a.left < input.safe_left - 0.001
                || a.right() > input.width - input.safe_right + 0.001
                || a.top < top
                || a.bottom() > input.height - 20.0 + 0.001
            {
                return Err(format!("{name} out of bounds: {a:?}"));
            }
            Ok(())
        };
        let core = [
            ("map", p.minimap),
            ("allies", p.allies),
            ("target", p.target),
            (
                "score-hit",
                Box2::new(p.score.left, p.score.top, p.score.width, 44.0),
            ),
            ("menu", p.menu),
            ("chat", p.chat),
            ("buy", p.quick_buy),
            ("feed", p.kill_feed),
            ("brush", p.brush),
            ("status", p.player_status),
            ("feedback", p.feedback),
        ];
        for (i, (name, a)) in core.iter().enumerate() {
            bounds(name, *a)?;
            for (other, b) in core.iter().skip(i + 1) {
                if a.overlaps(*b) {
                    return Err(format!("{name} overlaps {other}: {a:?}/{b:?}"));
                }
            }
            for b in p.deaths.into_iter().chain(p.live_slots()) {
                if a.overlaps(b) {
                    return Err(format!("{name} overlaps portrait {b:?}"));
                }
            }
        }
        for slot in p.deaths.into_iter().chain(p.live_slots()) {
            bounds("portrait", slot)?;
        }
        for (i, a) in p.deaths.iter().enumerate() {
            if a.top != p.allies.bottom() + GAP || a.width != DEATH_CELL || a.height != DEATH_CELL {
                return Err("death row detached from allies".into());
            }
            for b in p.deaths.iter().skip(i + 1).copied().chain(p.live_slots()) {
                if a.overlaps(b) {
                    return Err("death/live slots overlap".into());
                }
            }
        }
        for (i, cell) in p.ally_cells().iter().enumerate() {
            bounds("ally cell", *cell)?;
            if cell.top != top
                || cell.width != 32.0
                || cell.height != 44.0
                || cell.left < p.allies.left
                || cell.right() > p.allies.right() + 0.001
                || cell.bottom() > p.allies.bottom() + 0.001
            {
                return Err("real ally cell does not fit its region".into());
            }
            for b in p.ally_cells().iter().skip(i + 1) {
                if cell.overlaps(*b) {
                    return Err("ally hit cells overlap".into());
                }
            }
        }
        if p.chat.top != p.menu.top
            || p.chat.left != p.menu.right()
            || p.menu.top < p.deaths[0].bottom() + GAP
        {
            return Err("settings/chat do not follow death row".into());
        }
        if p.kill_feed.top != p.quick_buy.bottom() + GAP
            || p.quick_buy.top != p.minimap.bottom() + GAP
        {
            return Err("left stack detached from actual map".into());
        }
        if input.height - 20.0 - (p.kill_feed.bottom() + GAP) < 44.0 {
            return Err("no usable movement strip remains".into());
        }
        Ok(())
    }
    #[test]
    fn oppo_keeps_both_top_anchors_and_uses_a_smaller_target() {
        for inset in [32.0, 47.0] {
            let i = phone(791.0, 360.0, inset, inset, 4.0);
            let p = solve(i);
            contract(i, p).unwrap();
            assert_eq!(p.target.width, 144.0);
            assert_eq!(p.target.height, 40.0);
            assert_eq!(p.minimap.top, p.allies.top);
            assert_eq!(p.minimap.top, 4.0);
        }
    }
    #[test]
    fn old_795_796_cutoff_never_drops_either_header_block() {
        for width in [790.0, 791.0, 795.0, 796.0, 797.0, 844.0] {
            let i = phone(width, 360.0, 32.0, 32.0, 4.0);
            contract(i, solve(i)).unwrap();
        }
    }
    #[test]
    fn narrow_fallback_moves_map_instead_of_avatars_or_target() {
        for width in [568.0, 693.0] {
            for inset in [32.0, 47.0] {
                let i = phone(width, 320.0, inset, inset, 4.0);
                let p = solve(i);
                contract(i, p).unwrap();
                assert!(p.minimap.top > p.allies.top);
                assert_eq!(p.minimap.top, p.allies.bottom() + 4.0);
                assert_eq!(p.minimap.width, 104.0);
            }
        }
    }
    #[test]
    fn deterministic_landscape_width_and_safe_inset_contract_grid() {
        for width in (568..=1440).step_by(7) {
            for height in [320.0, 360.0, 384.0, 600.0] {
                if height > width as f32 {
                    continue;
                }
                for left in [0.0, 32.0, 47.0] {
                    for right in [0.0, 32.0, 47.0] {
                        for top in [0.0, 4.0, 12.0] {
                            let i = phone(width as f32, height, left, right, top);
                            contract(i, solve(i)).unwrap_or_else(|e| panic!("{i:?}: {e}"));
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn rejects_the_exact_lowered_avatar_regression_even_when_nothing_overlaps() {
        let i = phone(791.0, 360.0, 32.0, 32.0, 4.0);
        let mut p = solve(i);
        p.allies = Box2::new(214.0, 52.0, 134.0, 44.0);
        assert!(
            contract(i, p)
                .unwrap_err()
                .contains("allied portraits dropped")
        );
    }
    #[test]
    fn rejects_target_displacement_and_overlapping_real_ally_cells() {
        let i = phone(791.0, 360.0, 32.0, 32.0, 4.0);
        let mut p = solve(i);
        p.target.top += 48.0;
        assert!(contract(i, p).unwrap_err().contains("target dropped"));
        let mut p = solve(i);
        p.target.left += 8.0;
        assert!(contract(i, p).unwrap_err().contains("horizontally"));
        let mut p = solve(i);
        p.allies.width -= 4.0;
        assert!(contract(i, p).is_err());
    }
    #[test]
    fn rejects_offscreen_empty_portraits_and_cells_overflowing_the_ally_row() {
        let i = phone(791.0, 360.0, 32.0, 32.0, 4.0);
        let mut p = solve(i);
        p.deaths[0].left = i.width + 1.0;
        assert!(contract(i, p).is_err());
        let mut p = solve(i);
        p.deaths[0].height = 0.0;
        assert!(contract(i, p).is_err());
        let mut p = solve(i);
        p.allies.height = 0.0;
        assert!(contract(i, p).is_err());
        let mut p = solve(i);
        p.allies.height -= 4.0;
        for death in &mut p.deaths {
            death.top -= 4.0;
        }
        assert!(contract(i, p).is_err());
    }
    #[test]
    fn live_target_slots_stay_in_the_existing_right_side_positions() {
        let p = solve(phone(791.0, 360.0, 32.0, 32.0, 4.0));
        assert_eq!(p.live_slots()[0], Box2::new(705.0, 52.0, 44.0, 44.0));
        assert_eq!(p.live_slots()[4], Box2::new(609.0, 52.0, 44.0, 44.0));
        assert_eq!(p.deaths[0], Box2::new(168.0, 52.0, 32.0, 32.0));
        assert_eq!(p.menu.top, 88.0);
        assert_eq!(p.chat.top, 88.0);
    }
}
