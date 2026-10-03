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
        if link == nil {
            let displayLink = CADisplayLink(target: self, selector: #selector(frame))
            displayLink.add(to: .main, forMode: .common)
            link = displayLink
        }
        let limit = min(requested == 120 ? 120 : 60, UIScreen.main.maximumFramesPerSecond)
        // The OS may choose less in Low Power Mode or under thermal pressure.
        link?.preferredFrameRateRange = CAFrameRateRange(
            minimum: Float(min(30, limit)), maximum: Float(limit), preferred: Float(limit)
        )
        link?.isPaused = UIApplication.shared.applicationState != .active
    }

    @objc private func frame() { tick?() }
}

@_cdecl("omoba_frame_pacing_set")
public func omobaFramePacingSet(_ fps: Int32, _ callback: @escaping @convention(c) () -> Void) {
    guard Thread.isMainThread else { return }
    OmobaFramePacer.shared.configure(Int(fps), callback: callback)
}
