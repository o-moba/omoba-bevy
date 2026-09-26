//! Controller legends: button names for the hotbar, the aim label and a
//! one-strip control reminder, in PlayStation or generic (Xbox-style) names.
use bevy::{prelude::*, window::PrimaryWindow};

use super::GamepadControls;

/// Hotbar slot captions for skills 1–3 and the ultimate.
pub(crate) fn skill_labels(playstation: bool) -> [&'static str; 4] {
    if playstation {
        ["L1", "R1", "L2", "L2+R2"]
    } else {
        ["LB", "RB", "LT", "LT+RT"]
    }
}

/// The aim reticle's caption while the controller owns input.
pub(crate) fn aim_label(playstation: bool, aiming: bool, locked: bool) -> &'static str {
    match (aiming, locked, playstation) {
        (true, _, _) => "RELEASE TO CAST",
        (false, true, _) => "TARGET LOCKED",
        (false, false, true) => "R2 ATTACK · R3 LOCK",
        (false, false, false) => "RT ATTACK · RS LOCK",
    }
}

/// The reminder strip: menu controls on menus, the full layout in a match.
pub(crate) fn legend(playstation: bool, menu: bool, phone: bool) -> &'static str {
    match (playstation, menu, phone) {
        (true, true, _) => "D-pad / LS: navigate · Cross: select · Circle: back",
        (false, true, _) => "D-pad / LS: navigate · A: select · B: back",
        // The phone skill bar carries the four trigger labels above this strip.
        (true, false, true) => {
            "LS move · RS aim · R3 lock · R2 attack · Circle cancel · Options menu\nHold skill: aim; release: cast · Triangle + skill: upgrade · D-pad right: shop; left: reactions"
        }
        (false, false, true) => {
            "LS move · RS aim · RS click lock · RT attack · B cancel · Menu\nHold skill: aim; release: cast · Y + skill: upgrade · D-pad right: shop; left: reactions"
        }
        (true, false, false) => {
            "Left stick: move   Right stick: aim   R2: attack   R3: lock\nHold L1 / R1 / L2: aim skill, release: cast   L2+R2: ultimate\nCircle: cancel   Triangle + skill: upgrade   Options: menu   D-pad right: shop; left: reactions"
        }
        (false, false, false) => {
            "Left stick: move   Right stick: aim   RT: attack   RS click: lock\nHold LB / RB / LT: aim skill, release: cast   LT+RT: ultimate\nB: cancel   Y + skill: upgrade   Menu: menu   D-pad right: shop; left: reactions"
        }
    }
}

#[derive(Component)]
pub(crate) struct ControllerLegend;

pub(crate) fn setup_legend(mut commands: Commands) {
    commands.spawn((
        Text::new(""),
        crate::ui::theme::text(12.0),
        TextColor(crate::ui::theme::IVORY),
        BackgroundColor(crate::ui::theme::PANEL),
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(4.0),
            left: Val::Percent(20.0),
            max_width: Val::Percent(60.0),
            padding: UiRect::axes(Val::Px(8.0), Val::Px(5.0)),
            display: Display::None,
            ..default()
        },
        GlobalZIndex(2000),
        bevy::ui::FocusPolicy::Pass,
        Pickable::IGNORE,
        ControllerLegend,
        Name::new("ControllerLegend"),
    ));
}

/// Shows the strip while the controller owns input: along the bottom in a
/// match, a one-line hint at the top on menus.
pub(crate) fn draw_legend(
    controls: Res<GamepadControls>,
    context: Res<crate::input_context::GameplayInputContext>,
    mobile: Option<Res<crate::mobile_controls::MobileControls>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut strips: Query<(&mut Node, &mut Text, &mut TextFont), With<ControllerLegend>>,
) {
    let focused = windows.single().is_ok_and(|window| window.focused);
    let visible = controls.active && controls.connected && focused;
    let phone = mobile.as_ref().filter(|mobile| mobile.enabled);
    let menu = !context.gameplay_allowed();
    for (mut node, mut text, mut font) in &mut strips {
        let display = if visible {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
        if !visible {
            continue;
        }
        let (top, bottom, size) = if menu {
            let top = phone.map_or(12.0, |mobile| mobile.safe.top + 8.0);
            (Val::Px(top), Val::Auto, 10.0)
        } else if let Some(mobile) = phone {
            (Val::Auto, Val::Px(mobile.safe.bottom), 10.0)
        } else {
            (Val::Auto, Val::Px(4.0), 12.0)
        };
        if node.top != top || node.bottom != bottom {
            node.top = top;
            node.bottom = bottom;
        }
        if font.font_size != size {
            font.font_size = size;
        }
        let value = legend(controls.playstation, menu, phone.is_some());
        if text.0 != value {
            text.0 = value.to_owned();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_match_controller_family_and_explain_upgrade_modifier() {
        assert_eq!(skill_labels(true), ["L1", "R1", "L2", "L2+R2"]);
        assert_eq!(skill_labels(false), ["LB", "RB", "LT", "LT+RT"]);
        assert!(legend(true, true, false).contains("Cross"));
        assert!(legend(false, true, false).contains("A: select"));
        assert!(legend(true, false, false).contains("Triangle + skill"));
        assert!(legend(false, false, false).contains("Y + skill"));
        for playstation in [true, false] {
            assert_eq!(legend(playstation, false, true).lines().count(), 2);
            assert!(
                legend(playstation, false, true).contains("D-pad right: shop; left: reactions")
            );
        }
        assert_eq!(aim_label(true, true, true), "RELEASE TO CAST");
        assert_eq!(aim_label(false, false, true), "TARGET LOCKED");
        assert!(aim_label(true, false, false).starts_with("R2"));
        assert!(aim_label(false, false, false).starts_with("RT"));
    }
}
