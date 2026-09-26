//! One normalized controller snapshot per frame, whatever the platform.
//!
//! Desktop reads Bevy's `Gamepad` (the gilrs backend in Bevy's default
//! features); iOS polls Apple's GameController framework through
//! [`super::ios`] because gilrs has no iOS backend. Both produce the same
//! [`PadSnapshot`]: a 16-bit held-button mask, dead-zoned sticks with +Y up,
//! a process-local device identity and the controller family.
use bevy::prelude::*;

pub(crate) const SOUTH: u32 = 1 << 0;
pub(crate) const EAST: u32 = 1 << 1;
pub(crate) const WEST: u32 = 1 << 2;
pub(crate) const NORTH: u32 = 1 << 3;
pub(crate) const L1: u32 = 1 << 4;
pub(crate) const R1: u32 = 1 << 5;
pub(crate) const L2: u32 = 1 << 6;
pub(crate) const R2: u32 = 1 << 7;
pub(crate) const L3: u32 = 1 << 8;
pub(crate) const R3: u32 = 1 << 9;
pub(crate) const UP: u32 = 1 << 10;
pub(crate) const DOWN: u32 = 1 << 11;
pub(crate) const LEFT: u32 = 1 << 12;
pub(crate) const RIGHT: u32 = 1 << 13;
/// Primary menu button: Options on PlayStation, Menu/Start elsewhere.
pub(crate) const START: u32 = 1 << 14;
/// Secondary menu button: Create/Share on PlayStation, View/Select elsewhere.
pub(crate) const SELECT: u32 = 1 << 15;
/// Every defined bit; anything else a source reports is dropped.
pub(crate) const BUTTON_MASK: u32 = SOUTH
    | EAST
    | WEST
    | NORTH
    | L1
    | R1
    | L2
    | R2
    | L3
    | R3
    | UP
    | DOWN
    | LEFT
    | RIGHT
    | START
    | SELECT;
/// The ultimate chord.
pub(crate) const CHORD: u32 = L2 | R2;

/// Radial dead zone: stick drift inside it reads as centred.
pub(crate) const DEAD_ZONE: f32 = 0.18;

/// Bevy buttons in bit order (bit `i` is `DESKTOP_BUTTONS[i]`).
#[cfg(not(target_os = "ios"))]
const DESKTOP_BUTTONS: [GamepadButton; 16] = [
    GamepadButton::South,
    GamepadButton::East,
    GamepadButton::West,
    GamepadButton::North,
    GamepadButton::LeftTrigger,
    GamepadButton::RightTrigger,
    GamepadButton::LeftTrigger2,
    GamepadButton::RightTrigger2,
    GamepadButton::LeftThumb,
    GamepadButton::RightThumb,
    GamepadButton::DPadUp,
    GamepadButton::DPadDown,
    GamepadButton::DPadLeft,
    GamepadButton::DPadRight,
    GamepadButton::Start,
    GamepadButton::Select,
];

#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub(crate) struct PadSnapshot {
    /// Nonzero, process-local; changes when the device is replaced.
    pub identity: u64,
    /// A DualShock/DualSense: PlayStation names in legends.
    pub playstation: bool,
    /// Dead-zoned, +X right, +Y up, length at most 1.
    pub left: Vec2,
    pub right: Vec2,
    /// Held buttons, the bit constants above.
    pub buttons: u32,
}

impl PadSnapshot {
    /// No button held and both sticks centred.
    pub(crate) fn neutral(&self) -> bool {
        self.buttons == 0 && self.left == Vec2::ZERO && self.right == Vec2::ZERO
    }
}

/// Removes drift inside [`DEAD_ZONE`], rescales the rest to 0..1 without
/// bending diagonals, and turns NaN/infinite readings into centred.
pub(crate) fn radial_dead_zone(value: Vec2) -> Vec2 {
    if !value.is_finite() {
        return Vec2::ZERO;
    }
    let length = value.length();
    if length <= DEAD_ZONE {
        return Vec2::ZERO;
    }
    value / length * ((length.min(1.0) - DEAD_ZONE) / (1.0 - DEAD_ZONE))
}

/// A raw axis from a bridge: clamped, and 0 when not finite.
pub(crate) fn clamp_axis(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

const SONY_VENDOR_ID: u16 = 0x054c;

/// PlayStation controllers by USB vendor id or by the OS-reported name.
pub(crate) fn is_playstation(vendor: Option<u16>, name: &str) -> bool {
    let name = name.to_lowercase();
    vendor == Some(SONY_VENDOR_ID)
        || [
            "sony",
            "dualsense",
            "dualshock",
            "playstation",
            "ps4",
            "ps5",
        ]
        .iter()
        .any(|family| name.contains(family))
}

/// The desktop source: the controller already in use while it stays
/// connected, else the one with the lowest entity index. Enumeration order never
/// switches controllers mid-match.
#[cfg(not(target_os = "ios"))]
pub(crate) fn desktop_snapshot(
    pads: &Query<(Entity, &Gamepad, Option<&Name>)>,
    previous: Option<u64>,
) -> Option<PadSnapshot> {
    let (entity, pad, name) = pads
        .iter()
        .find(|(entity, ..)| Some(entity.to_bits()) == previous)
        .or_else(|| pads.iter().min_by_key(|(entity, ..)| entity.index_u32()))?;
    let buttons = DESKTOP_BUTTONS
        .iter()
        .enumerate()
        .filter(|(_, button)| pad.pressed(**button))
        .fold(0, |bits, (index, _)| bits | 1 << index)
        & BUTTON_MASK;
    let axis = |v: Vec2| radial_dead_zone(Vec2::new(clamp_axis(v.x), clamp_axis(v.y)));
    Some(PadSnapshot {
        identity: entity.to_bits(),
        playstation: is_playstation(pad.vendor_id(), name.map_or("", Name::as_str)),
        left: axis(pad.left_stick()),
        right: axis(pad.right_stick()),
        buttons,
    })
}

/// The iOS source: one copied value snapshot from the Swift bridge.
#[cfg(target_os = "ios")]
pub(crate) fn ios_snapshot() -> Option<PadSnapshot> {
    super::ios::poll().map(|pad| PadSnapshot {
        identity: pad.identity,
        playstation: pad.is_playstation,
        left: radial_dead_zone(Vec2::from_array(pad.left_stick)),
        right: radial_dead_zone(Vec2::from_array(pad.right_stick)),
        buttons: pad.buttons,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radial_filter_removes_drift_preserves_diagonal_and_rejects_nan() {
        assert_eq!(radial_dead_zone(Vec2::splat(0.1)), Vec2::ZERO);
        assert_eq!(radial_dead_zone(Vec2::new(f32::NAN, 0.0)), Vec2::ZERO);
        assert_eq!(radial_dead_zone(Vec2::new(f32::INFINITY, 1.0)), Vec2::ZERO);
        let diagonal = radial_dead_zone(Vec2::ONE);
        assert!((diagonal.length() - 1.0).abs() < 1e-6, "clamped to 1");
        assert!((diagonal.x - diagonal.y).abs() < 1e-6, "direction kept");
        let v = radial_dead_zone(Vec2::X * 0.59);
        assert!((v.x - 0.5).abs() < 1e-5, "rescaled past the dead zone");
        assert_eq!(clamp_axis(f32::NAN), 0.0);
        assert_eq!(clamp_axis(-3.0), -1.0);
    }

    #[test]
    fn button_layout_and_family_detection() {
        assert_eq!(SOUTH, 1);
        assert_eq!(SELECT, 1 << 15);
        assert_eq!(BUTTON_MASK.count_ones(), 16);
        assert_eq!(CHORD, L2 | R2);
        assert!(is_playstation(Some(0x054c), "Wireless Controller"));
        assert!(is_playstation(None, "DualSense Wireless Controller"));
        assert!(!is_playstation(Some(0x045e), "Xbox Wireless Controller"));
        let pad = PadSnapshot::default();
        assert!(pad.neutral());
        assert!(
            !PadSnapshot {
                right: Vec2::X,
                ..pad
            }
            .neutral()
        );
    }

    #[cfg(not(target_os = "ios"))]
    #[test]
    fn desktop_source_maps_bevy_buttons_and_keeps_the_chosen_pad() {
        use bevy::ecs::system::RunSystemOnce;
        let mut app = App::new();
        let first = app.world_mut().spawn(Gamepad::default()).id();
        let second = app
            .world_mut()
            .spawn((
                Gamepad::default(),
                Name::new("DualSense Wireless Controller"),
            ))
            .id();
        {
            let mut pad = app.world_mut().get_mut::<Gamepad>(second).unwrap();
            pad.digital_mut().press(GamepadButton::RightTrigger2);
            pad.digital_mut().press(GamepadButton::Start);
            pad.analog_mut().set(GamepadAxis::LeftStickX, 1.0);
            pad.analog_mut().set(GamepadAxis::LeftStickY, 0.1);
        }
        let read = move |app: &mut App, previous: Option<u64>| {
            app.world_mut()
                .run_system_once(move |pads: Query<(Entity, &Gamepad, Option<&Name>)>| {
                    desktop_snapshot(&pads, previous)
                })
                .unwrap()
        };
        let oldest = read(&mut app, None).unwrap();
        assert_eq!(oldest.identity, first.to_bits());
        assert!(!oldest.playstation && oldest.neutral());
        let kept = read(&mut app, Some(second.to_bits())).unwrap();
        assert_eq!(kept.identity, second.to_bits());
        assert!(kept.playstation);
        assert_eq!(kept.buttons, R2 | START);
        assert!(kept.left.x > 0.9 && kept.left.y > 0.0);
        app.world_mut().entity_mut(first).remove::<Gamepad>();
        app.world_mut().entity_mut(second).remove::<Gamepad>();
        assert_eq!(read(&mut app, Some(second.to_bits())), None);
    }
}
