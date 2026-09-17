//! Main-thread polling of Apple's GameController framework.
//!
//! No Swift objects or pointers escape the call. A changed identity, `None`, or
//! a game focus transition must cancel held gameplay actions in the caller.

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
pub(crate) const DPAD_UP: u32 = 1 << 10;
pub(crate) const DPAD_DOWN: u32 = 1 << 11;
pub(crate) const DPAD_LEFT: u32 = 1 << 12;
pub(crate) const DPAD_RIGHT: u32 = 1 << 13;
/// Primary menu button: Options on PlayStation controllers.
pub(crate) const START: u32 = 1 << 14;
/// Secondary menu button: Share/Create on PlayStation controllers.
pub(crate) const SELECT: u32 = 1 << 15;

const BUTTON_MASK: u32 = SOUTH
    | EAST
    | WEST
    | NORTH
    | L1
    | R1
    | L2
    | R2
    | L3
    | R3
    | DPAD_UP
    | DPAD_DOWN
    | DPAD_LEFT
    | DPAD_RIGHT
    | START
    | SELECT;

#[derive(Clone, Copy, Debug)]
pub(crate) struct IosGamepadSnapshot {
    /// Nonzero, process-local generation. Changes after disconnect/replacement
    /// and after the app becomes inactive, even between successive polls.
    pub identity: u64,
    pub is_playstation: bool,
    /// Normalized axes: +X right, +Y up; dead zones are applied by the caller.
    pub left_stick: [f32; 2],
    pub right_stick: [f32; 2],
    /// Current held state using the constants above, not queued press events.
    pub buttons: u32,
}

unsafe extern "C" {
    fn omoba_gamecontroller_poll(
        identity: *mut u64,
        is_playstation: *mut u32,
        axes: *mut f32,
        axis_capacity: i32,
        buttons: *mut u32,
    ) -> i32;
}

/// Call from a main-thread Bevy system (with a `NonSend` parameter).
/// Returns `None` off the main thread, while the app is inactive, or without an
/// extended gamepad. Polling is synchronous and never dispatches or blocks.
pub(crate) fn poll() -> Option<IosGamepadSnapshot> {
    let mut identity = 0;
    let mut is_playstation = 0;
    let mut axes = [0.0; 4];
    let mut buttons = 0;
    // SAFETY: all output pointers refer to initialized, writable stack storage;
    // Swift checks capacity, copies synchronously, and never retains pointers.
    let connected = unsafe {
        omoba_gamecontroller_poll(
            &mut identity,
            &mut is_playstation,
            axes.as_mut_ptr(),
            axes.len() as i32,
            &mut buttons,
        )
    };
    if connected != 1 || identity == 0 {
        return None;
    }
    let axes = axes.map(|axis| {
        if axis.is_finite() {
            axis.clamp(-1.0, 1.0)
        } else {
            0.0
        }
    });
    Some(IosGamepadSnapshot {
        identity,
        is_playstation: is_playstation != 0,
        left_stick: [axes[0], axes[1]],
        right_stick: [axes[2], axes[3]],
        buttons: buttons & BUTTON_MASK,
    })
}
