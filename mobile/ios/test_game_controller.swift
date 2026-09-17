// Native regression harness (macOS):
// xcrun swiftc -swift-version 5 -target arm64-apple-macos14.0 \
//   mobile/ios/OmobaGameController.swift mobile/ios/test_game_controller.swift \
//   -o /tmp/omoba-game-controller-tests && /tmp/omoba-game-controller-tests
import Foundation
import GameController

@main
struct OmobaGameControllerTests {
    static func main() {
        let state = OmobaGameControllerState()
        let first = GCController.withExtendedGamepad()
        let second = GCController.withExtendedGamepad()
        let micro = GCController.withMicroGamepad()
        let profile = first.extendedGamepad!

        assert(state.read(controllers: [], isActive: true) == nil)
        assert(state.read(controllers: [micro], isActive: true) == nil)
        profile.leftThumbstick.xAxis.setValue(0.75)
        profile.leftThumbstick.yAxis.setValue(-0.5)
        profile.rightThumbstick.xAxis.setValue(-0.25)
        profile.rightThumbstick.yAxis.setValue(1)
        let initial = state.read(controllers: [micro, first], isActive: true)!
        assert(initial.identity != 0)
        assert(initial.axes == [0.75, -0.5, -0.25, 1])
        assert(initial.buttons == 0)
        assert(!initial.isPlayStation)

        // Enumeration order must not switch active controllers while connected.
        let reordered = state.read(controllers: [second, first], isActive: true)!
        assert(reordered.identity == initial.identity)
        assert(reordered.axes == initial.axes)

        // Exercise actual Apple snapshot/capture behavior, including optional
        // stick and secondary-menu buttons that can differ across controllers.
        let inputs: [(GCControllerButtonInput?, UInt32)] = [
            (profile.buttonA, 1 << 0), (profile.buttonB, 1 << 1),
            (profile.buttonX, 1 << 2), (profile.buttonY, 1 << 3),
            (profile.leftShoulder, 1 << 4), (profile.rightShoulder, 1 << 5),
            (profile.leftTrigger, 1 << 6), (profile.rightTrigger, 1 << 7),
            (profile.leftThumbstickButton, 1 << 8), (profile.rightThumbstickButton, 1 << 9),
            (profile.buttonMenu, 1 << 14), (profile.buttonOptions, 1 << 15),
        ]
        var testedButtons = 0
        for (input, expected) in inputs {
            guard let input else { continue }
            input.setValue(1)
            let actual = state.read(controllers: [first], isActive: true)!.buttons
            assert(actual == expected, "Button mask \(expected): received \(actual)")
            input.setValue(0)
            assert(state.read(controllers: [first], isActive: true)!.buttons == 0)
            testedButtons += 1
        }
        // Apple capture() copies directional-pad axes. Write through that API,
        // as changing an individual virtual button does not update its axis.
        for (x, y, expected): (Float, Float, UInt32) in [
            (0, 1, 1 << 10), (0, -1, 1 << 11),
            (-1, 0, 1 << 12), (1, 0, 1 << 13),
        ] {
            profile.dpad.setValueForXAxis(x, yAxis: y)
            assert(state.read(controllers: [first], isActive: true)!.buttons == expected)
            testedButtons += 1
        }
        profile.dpad.setValueForXAxis(0, yAxis: 0)
        profile.leftTrigger.setValue(1)
        profile.rightTrigger.setValue(1)
        assert(state.read(controllers: [first], isActive: true)!.buttons == (1 << 6) | (1 << 7))

        // Background/foreground and disconnect/reconnect create new identities
        // even when the exact same controller object is subsequently returned.
        assert(state.read(controllers: [first], isActive: false) == nil)
        let resumed = state.read(controllers: [first], isActive: true)!
        assert(resumed.identity != initial.identity)
        assert(state.read(controllers: [], isActive: true) == nil)
        let reconnected = state.read(controllers: [first], isActive: true)!
        assert(reconnected.identity != resumed.identity)
        state.invalidate()
        let afterNotification = state.read(controllers: [first], isActive: true)!
        assert(afterNotification.identity != reconnected.identity)
        let replaced = state.read(controllers: [second], isActive: true)!
        assert(replaced.identity != afterNotification.identity)
        assert(replaced.axes == [0, 0, 0, 0] && replaced.buttons == 0)

        // A short buffer is rejected before touching memory; nils are accepted.
        var identity: UInt64 = 123
        var vendor: UInt32 = 123
        var axes: [Float] = [123, 123, 123, 123]
        var buttons: UInt32 = 123
        let short = axes.withUnsafeMutableBufferPointer {
            omobaGameControllerPoll(&identity, &vendor, $0.baseAddress, 3, &buttons)
        }
        assert(short == 0 && identity == 123 && axes == [123, 123, 123, 123])
        assert(omobaGameControllerPoll(nil, nil, nil, 4, nil) == 0)

        // Off-main calls fail closed and clear stale outputs without blocking on
        // the main queue (which is deliberately waiting for this worker).
        let worker = Thread {
            var identity: UInt64 = 123
            var vendor: UInt32 = 123
            var axes: [Float] = [123, 123, 123, 123]
            var buttons: UInt32 = 123
            let result = axes.withUnsafeMutableBufferPointer {
                omobaGameControllerPoll(&identity, &vendor, $0.baseAddress, 4, &buttons)
            }
            assert(result == 0 && identity == 0 && vendor == 0 && buttons == 0)
            assert(axes == [0, 0, 0, 0])
        }
        worker.start()
        while !worker.isFinished { Thread.sleep(forTimeInterval: 0.001) }
        print("PASS: Apple GameController axes, \(testedButtons) buttons, chord, stable selection, lifecycle, disconnect, replacement and C ABI guards")
    }
}
