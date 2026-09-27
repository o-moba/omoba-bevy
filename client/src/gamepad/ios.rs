//! Main-thread polling of Apple's GameController framework
//! (`mobile/ios/OmobaGameController.swift`, linked by `client/build.rs`).
//!
//! No Swift object or pointer escapes the call: the bridge copies one value
//! snapshot into caller-owned stack storage. A changed identity or `None`
//! (disconnect, replacement, the app resigning active) makes the caller
//! cancel held gameplay actions.
// i18n-strict
use super::snapshot::{BUTTON_MASK, clamp_axis};

#[derive(Clone, Copy, Debug)]
pub(crate) struct IosGamepadSnapshot {
    /// Nonzero, process-local generation. Changes after disconnect,
    /// replacement and after the app becomes inactive.
    pub identity: u64,
    pub is_playstation: bool,
    /// Clamped axes: +X right, +Y up; the caller applies the dead zone.
    pub left_stick: [f32; 2],
    pub right_stick: [f32; 2],
    /// Held buttons in the `snapshot` bit layout.
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

/// Call from a main-thread system (the sampler holds `NonSendMarker`).
/// `None` off the main thread, while the app is inactive, or without an
/// extended gamepad. Synchronous; never dispatches or blocks.
pub(crate) fn poll() -> Option<IosGamepadSnapshot> {
    let mut identity = 0;
    let mut is_playstation = 0;
    let mut axes = [0.0f32; 4];
    let mut buttons = 0;
    // SAFETY: every pointer refers to initialized, writable stack storage of
    // the advertised capacity; Swift copies synchronously and retains nothing.
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
    let axes = axes.map(clamp_axis);
    Some(IosGamepadSnapshot {
        identity,
        is_playstation: is_playstation != 0,
        left_stick: [axes[0], axes[1]],
        right_stick: [axes[2], axes[3]],
        buttons: buttons & BUTTON_MASK,
    })
}
