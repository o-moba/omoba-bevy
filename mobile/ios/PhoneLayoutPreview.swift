// Temporary iPad layout preview. Resize the UIWindow itself: winit's iOS
// layoutSubviews follows window.bounds and its touches use window coordinates.
import Foundation
import UIKit

private final class PhonePreviewBackdrop: UIViewController {
    override var prefersStatusBarHidden: Bool { true }
    override var supportedInterfaceOrientations: UIInterfaceOrientationMask { .landscape }
}

private final class PhoneLayoutPreview {
    static let shared = PhoneLayoutPreview()
    private let phone = CGSize(width: 852, height: 393)
    private weak var game: UIWindow?
    private var backdrop: UIWindow?
    private var originalFrame = CGRect.zero
    private var screenBounds = CGRect.zero
    private var previewFrame = CGRect.zero
    private var timer: Timer?
    private var observers: [NSObjectProtocol] = []
    private var reapplied = 0
    private var reappliedTotal = 0
    private var reapplyWindowStart: TimeInterval = 0
    var active: Bool { backdrop != nil }

    private init() {
        // Only losing the foreground ends the preview from a notification.
        // UIDevice orientation notifications also arrive for face-up,
        // face-down and portrait tilts that never rotate this landscape-only
        // interface, and each of them used to end the preview on a hand-held
        // iPad. validate() notices a host whose geometry really changed.
        observers.append(NotificationCenter.default.addObserver(
            forName: UIApplication.willResignActiveNotification, object: nil, queue: .main
        ) { [weak self] _ in self?.restore("the application resigned active") })
    }

    private func gameWindow() -> UIWindow? {
        guard let viewClass = NSClassFromString("WinitUIView") else { return nil }
        // winit currently uses a legacy UIWindow without a UIWindowScene.
        let sceneWindows = UIApplication.shared.connectedScenes
            .compactMap { $0 as? UIWindowScene }.flatMap { $0.windows }
        return (sceneWindows + UIApplication.shared.windows).first {
            !$0.isHidden && $0.rootViewController?.view.isKind(of: viewClass) == true
        }
    }

    private func hostBounds(_ window: UIWindow) -> CGRect {
        window.windowScene?.coordinateSpace.bounds ?? window.screen.bounds
    }

    /// Why this host cannot show the preview, or nil when it can.
    private func ineligibility(_ window: UIWindow, _ frame: CGRect) -> String? {
        if UIDevice.current.userInterfaceIdiom != .pad { return "the device is not an iPad" }
        if UIApplication.shared.applicationState != .active { return "the application is not active" }
        if !window.transform.isIdentity { return "the game window is transformed" }
        if frame.width < phone.width || frame.height < phone.height + 160 || frame.width < frame.height {
            return "the host \(frame.size) is too small or not landscape"
        }
        if window.rootViewController?.view.transform.isIdentity != true { return "the game view is transformed" }
        return nil
    }

    private func eligible(_ window: UIWindow, _ frame: CGRect) -> Bool {
        ineligibility(window, frame) == nil
    }

    var available: Bool {
        guard let window = game ?? gameWindow() else { return false }
        return eligible(window, active ? originalFrame : window.frame)
    }

    func set(_ enabled: Bool, title: String) {
        guard enabled else { restore("the player left the preview"); return }
        guard !active, let window = gameWindow(), eligible(window, window.frame) else { return }
        game = window
        originalFrame = window.frame
        screenBounds = hostBounds(window)
        previewFrame = CGRect(
            x: originalFrame.midX - phone.width / 2,
            y: originalFrame.midY - phone.height / 2,
            width: phone.width, height: phone.height
        )
        let background: UIWindow
        if let scene = window.windowScene { background = UIWindow(windowScene: scene) }
        else { background = UIWindow(frame: originalFrame); background.screen = window.screen }
        background.frame = originalFrame
        background.windowLevel = UIWindow.Level(rawValue: window.windowLevel.rawValue - 1)
        let controller = PhonePreviewBackdrop()
        controller.view.backgroundColor = .black
        let button = UIButton(type: .system)
        var style = UIButton.Configuration.filled()
        style.title = title
        style.baseBackgroundColor = .darkGray
        style.baseForegroundColor = .white
        button.configuration = style
        button.accessibilityIdentifier = "PhonePreviewReturnToIPad"
        button.frame = CGRect(x: 20, y: 20, width: min(originalFrame.width - 40, 340), height: 48)
        button.addAction(UIAction { [weak self] _ in
            self?.restore("the player pressed the return button")
        }, for: .touchUpInside)
        controller.view.addSubview(button)
        background.rootViewController = controller
        backdrop = background
        // Keep the game's key window and its input focus. The lower window
        // receives only points outside the centered game's window rectangle.
        background.isHidden = false
        reapplied = 0
        reappliedTotal = 0
        applyPreviewFrame(window)
        timer = Timer.scheduledTimer(withTimeInterval: 0.2, repeats: true) { [weak self] _ in
            self?.validate()
        }
        validate()
    }

    private func applyPreviewFrame(_ window: UIWindow) {
        window.frame = previewFrame
        window.rootViewController?.view.frame = window.bounds
        window.layoutIfNeeded()
    }

    func validate() {
        guard active else { return }
        guard let window = game, !window.isHidden else { restore("the game window is gone"); return }
        if let reason = ineligibility(window, originalFrame) { restore(reason); return }
        guard hostBounds(window) == screenBounds else { restore("the host geometry changed"); return }
        if window.frame == previewFrame, window.bounds.size == phone,
           window.rootViewController?.view.bounds.size == phone { return }
        // UIKit laid the window out again while the host stayed the same
        // (a scene refresh, the keyboard). Put the preview back; a host that
        // keeps undoing it, in a burst or slowly for the whole session, ends
        // the preview instead of fighting it.
        let now = ProcessInfo.processInfo.systemUptime
        if now - reapplyWindowStart > 2 {
            reapplyWindowStart = now
            reapplied = 0
        }
        reapplied += 1
        reappliedTotal += 1
        guard reapplied <= 5, reappliedTotal <= 20 else { restore("the window frame kept changing"); return }
        applyPreviewFrame(window)
    }

    func restore(_ reason: String) {
        guard active else { return }
        // The only trace of why a preview ended on a device; keep it.
        NSLog("Omoba phone preview ended: %@", reason)
        timer?.invalidate()
        timer = nil
        if let window = game {
            // A rotation/scene resize restores today's full host, not stale dimensions.
            let host = hostBounds(window)
            window.frame = host == screenBounds ? originalFrame : host
            window.rootViewController?.view.frame = window.bounds
            window.layoutIfNeeded()
        }
        backdrop?.isHidden = true
        backdrop?.rootViewController = nil
        backdrop = nil
        game = nil
    }
}

// Call from the Rust main-thread system. Never synchronously dispatch UIKit
// from a Bevy worker: the main thread may already be waiting for that worker.
@_cdecl("omoba_phone_preview_available")
public func omobaPhonePreviewAvailable() -> Bool {
    guard Thread.isMainThread else { return false }
    return PhoneLayoutPreview.shared.available
}

@_cdecl("omoba_phone_preview_active")
public func omobaPhonePreviewActive() -> Bool {
    guard Thread.isMainThread else { return false }
    PhoneLayoutPreview.shared.validate()
    return PhoneLayoutPreview.shared.active
}

@_cdecl("omoba_phone_preview_set")
public func omobaPhonePreviewSet(_ enabled: Bool, _ returnTitle: UnsafePointer<CChar>?) {
    guard Thread.isMainThread else { return }
    PhoneLayoutPreview.shared.set(enabled, title: returnTitle.map { String(cString: $0) } ?? "Return to iPad")
}
