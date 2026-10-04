// Display-driven wakes for Bevy's reactive event loop. Input remains queued
// until the next display tick, so finger movement cannot defeat the FPS cap.
import Foundation
import QuartzCore
import UIKit

private final class OmobaFramePacer: NSObject {
    static let shared = OmobaFramePacer()
    private var link: CADisplayLink?
    private var tick: (@convention(c) () -> Void)?
    private var observers: [NSObjectProtocol] = []
    private var previousTimestamp: CFTimeInterval?
    private var sampledSeconds: CFTimeInterval = 0
    private var sampledFrames = 0
    private(set) var measuredHz: Double = 0

    override private init() {
        super.init()
        observers.append(NotificationCenter.default.addObserver(
            forName: UIApplication.willResignActiveNotification,
            object: nil, queue: .main
        ) { [weak self] _ in self?.link?.isPaused = true })
        observers.append(NotificationCenter.default.addObserver(
            forName: UIApplication.didBecomeActiveNotification,
            object: nil, queue: .main
        ) { [weak self] _ in self?.link?.isPaused = false })
    }

    func configure(_ requested: Int, callback: @escaping @convention(c) () -> Void) {
        tick = callback
        previousTimestamp = nil
        sampledSeconds = 0
        sampledFrames = 0
        measuredHz = 0
        if link == nil {
            let displayLink = CADisplayLink(target: self, selector: #selector(frame))
            displayLink.add(to: .main, forMode: .common)
            link = displayLink
        }
        let limit = min(requested == 120 ? 120 : 60, UIScreen.main.maximumFramesPerSecond)
        // Request the selected cadence, rather than permitting an unintended 30 Hz
        // floor during gameplay. The OS may still reduce it for power/thermal limits.
        link?.preferredFrameRateRange = CAFrameRateRange(
            minimum: Float(limit), maximum: Float(limit), preferred: Float(limit)
        )
        link?.isPaused = UIApplication.shared.applicationState != .active
    }

    @objc private func frame() {
        if let timestamp = link?.timestamp {
            if let previous = previousTimestamp {
                let delta = timestamp - previous
                if delta > 0 && delta <= 1 {
                    sampledSeconds += delta
                    sampledFrames += 1
                    if sampledSeconds >= 0.5 {
                        measuredHz = Double(sampledFrames) / sampledSeconds
                        sampledSeconds = 0
                        sampledFrames = 0
                    }
                } else {
                    sampledSeconds = 0
                    sampledFrames = 0
                    measuredHz = 0
                }
            }
            previousTimestamp = timestamp
        }
        tick?()
    }
}

@_cdecl("omoba_frame_pacing_set")
public func omobaFramePacingSet(_ fps: Int32, _ callback: @escaping @convention(c) () -> Void) {
    guard Thread.isMainThread else { return }
    OmobaFramePacer.shared.configure(Int(fps), callback: callback)
}

// Main-thread read-only diagnostics. Neither reports a requested rate as a
// measured rate; zero means the display link has not sampled enough frames.
@_cdecl("omoba_frame_pacing_max_fps")
public func omobaFramePacingMaximum() -> Int32 {
    guard Thread.isMainThread else { return 0 }
    return Int32(UIScreen.main.maximumFramesPerSecond)
}

@_cdecl("omoba_frame_pacing_hz")
public func omobaFramePacingMeasuredHz() -> Double {
    guard Thread.isMainThread else { return 0 }
    return OmobaFramePacer.shared.measuredHz
}
