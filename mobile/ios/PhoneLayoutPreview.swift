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
    var active: Bool { backdrop != nil }

    private init() {
        for name in [UIApplication.willResignActiveNotification,
                     UIDevice.orientationDidChangeNotification] {
            observers.append(NotificationCenter.default.addObserver(
                forName: name, object: nil, queue: .main
            ) { [weak self] _ in self?.restore() })
        }
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

    private func eligible(_ window: UIWindow, _ frame: CGRect) -> Bool {
        UIDevice.current.userInterfaceIdiom == .pad
            && UIApplication.shared.applicationState == .active
            && window.transform.isIdentity
            && frame.width >= phone.width && frame.height >= phone.height + 160
            && frame.width >= frame.height
            && window.rootViewController?.view.transform.isIdentity == true
    }

    var available: Bool {
        guard let window = game ?? gameWindow() else { return false }
        return eligible(window, active ? originalFrame : window.frame)
    }

    func set(_ enabled: Bool, title: String) {
        guard enabled else { restore(); return }
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
        button.addAction(UIAction { [weak self] _ in self?.restore() }, for: .touchUpInside)
        controller.view.addSubview(button)
        background.rootViewController = controller
        backdrop = background
        // Keep the game's key window and its input focus. The lower window
        // receives only points outside the centered game's window rectangle.
        background.isHidden = false
        window.frame = previewFrame
        window.rootViewController?.view.frame = window.bounds
        window.layoutIfNeeded()
        timer = Timer.scheduledTimer(withTimeInterval: 0.2, repeats: true) { [weak self] _ in
            self?.validate()
        }
        validate()
    }

    func validate() {
        guard active else { return }
        guard let window = game, eligible(window, originalFrame),
              !window.isHidden, window.frame == previewFrame,
              window.bounds.size == phone, hostBounds(window) == screenBounds,
              window.rootViewController?.view.bounds.size == phone else {
            restore(); return
        }
    }

    func restore() {
        guard active else { return }
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
