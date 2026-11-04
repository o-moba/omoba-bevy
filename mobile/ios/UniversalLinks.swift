import Foundation
import UIKit

@objc(OmobaLinkDelegate)
final class OmobaLinkDelegate: NSObject, UIApplicationDelegate {
    func application(_ application: UIApplication, continue userActivity: NSUserActivity,
                     restorationHandler: @escaping ([UIUserActivityRestoring]?) -> Void) -> Bool {
        guard userActivity.activityType == NSUserActivityTypeBrowsingWeb,
              let url = userActivity.webpageURL,
              let bundleID = Bundle.main.bundleIdentifier else { return false }
        // Foregrounding resumes the existing enrollment poll. No account is
        // approved, activated or switched by opening this link, including cold starts.
        return omobaAcceptsUniversalLink(url, bundleID: bundleID)
    }
}

// Referenced by Rust before UIApplicationMain so this Swift archive member and
// its Objective-C class metadata survive static linking and dead stripping.
// winit supports a custom delegate through the main nib and retains ownership
// of lifecycle notifications, windows and the event loop.
@_cdecl("omoba_universal_links_prepare")
public func omobaUniversalLinksPrepare() {
    _ = NSStringFromClass(OmobaLinkDelegate.self)
}
