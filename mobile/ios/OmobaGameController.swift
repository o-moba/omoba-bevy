// Value-only, main-thread GameController bridge. Keep the bit positions in sync
// with client/src/gamepad_ios.rs. No controller object crosses the C ABI.
import Foundation
import GameController
#if canImport(UIKit)
import UIKit
#endif

struct OmobaGameControllerSnapshot {
    let identity: UInt64
    let isPlayStation: Bool
    let axes: [Float]
    let buttons: UInt32
}

// Internal visibility lets the native tests exercise this state machine using
// Apple's writable GCController snapshots without physical controller hardware.
final class OmobaGameControllerState {
    private weak var selected: GCController?
    private var generation: UInt64 = 0

    func invalidate() {
        selected = nil
    }

    func read(controllers: [GCController], isActive: Bool) -> OmobaGameControllerSnapshot? {
        guard isActive else {
            invalidate()
            return nil
        }
        let connected = controllers.filter { $0.extendedGamepad != nil }
        let controller: GCController
        if let current = selected, connected.contains(where: { $0 === current }) {
            controller = current
        } else if let first = connected.first {
            selected = first
            controller = first
            generation &+= 1
            if generation == 0 { generation = 1 }
        } else {
            invalidate()
            return nil
        }
        // capture() gives one coherent value snapshot instead of mixing elements
        // from different input updates. The live object is checked every poll.
        guard let profile = controller.capture().extendedGamepad else { return nil }
        let isPlayStation = controller.extendedGamepad is GCDualShockGamepad
            || controller.extendedGamepad is GCDualSenseGamepad
        let inputs: [GCControllerButtonInput?] = [
            profile.buttonA, profile.buttonB, profile.buttonX, profile.buttonY,
            profile.leftShoulder, profile.rightShoulder,
            profile.leftTrigger, profile.rightTrigger,
            profile.leftThumbstickButton, profile.rightThumbstickButton,
            profile.dpad.up, profile.dpad.down, profile.dpad.left, profile.dpad.right,
            profile.buttonMenu, profile.buttonOptions,
        ]
        var buttons: UInt32 = 0
        for (index, button) in inputs.enumerated() where button?.isPressed == true {
            buttons |= UInt32(1) << index
        }
        let axes = [
            profile.leftThumbstick.xAxis.value, profile.leftThumbstick.yAxis.value,
            profile.rightThumbstick.xAxis.value, profile.rightThumbstick.yAxis.value,
        ].map { $0.isFinite ? min(1, max(-1, $0)) : 0 }
        return OmobaGameControllerSnapshot(
            identity: generation, isPlayStation: isPlayStation, axes: axes, buttons: buttons
        )
    }
}

// Created only after the C entry has checked Thread.isMainThread. UIKit and all
// controller selection state stay on that thread; background Rust callers fail
// closed rather than dispatching synchronously into a possible scheduler wait.
private final class OmobaGameControllerBridge {
    static let shared = OmobaGameControllerBridge()
    let state = OmobaGameControllerState()
    private var observers: [NSObjectProtocol] = []

    private init() {
        var invalidations: [Notification.Name] = [.GCControllerDidDisconnect]
        #if canImport(UIKit)
        invalidations.append(UIApplication.willResignActiveNotification)
        invalidations.append(UIApplication.didEnterBackgroundNotification)
        #endif
        for name in invalidations {
            observers.append(NotificationCenter.default.addObserver(
                forName: name, object: nil, queue: .main
            ) { [weak self] _ in
                // Also catches disconnect/reconnect or a focus round-trip that
                // happened entirely between polls while the game was suspended.
                self?.state.invalidate()
            })
        }
    }

    func poll() -> OmobaGameControllerSnapshot? {
        #if canImport(UIKit)
        let isActive = UIApplication.shared.applicationState == .active
        #else
        let isActive = true // The native macOS test harness has no UIApplication.
        #endif
        return state.read(controllers: GCController.controllers(), isActive: isActive)
    }
}

@_cdecl("omoba_gamecontroller_poll")
public func omobaGameControllerPoll(
    _ identity: UnsafeMutablePointer<UInt64>?,
    _ isPlayStation: UnsafeMutablePointer<UInt32>?,
    _ axes: UnsafeMutablePointer<Float>?,
    _ axisCapacity: Int32,
    _ buttons: UnsafeMutablePointer<UInt32>?
) -> Int32 {
    guard let identity, let isPlayStation, let axes, let buttons, axisCapacity >= 4 else { return 0 }
    identity.pointee = 0
    isPlayStation.pointee = 0
    buttons.pointee = 0
    for index in 0..<4 { axes[index] = 0 }
    guard Thread.isMainThread, let value = OmobaGameControllerBridge.shared.poll() else { return 0 }
    identity.pointee = value.identity
    isPlayStation.pointee = value.isPlayStation ? 1 : 0
    buttons.pointee = value.buttons
    for index in 0..<4 { axes[index] = value.axes[index] }
    return 1
}
