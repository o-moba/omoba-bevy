import Foundation

// A return link is only a wake-up hint. Authentication still comes from the
// signed, expiring enrollment on the Account API; URLs never supply credentials.
func omobaUniversalLinkHost(bundleID: String) -> String? {
    switch bundleID {
    case "space.ekza.omoba.beta": return "ios.omoba.ru"
    case "space.ekza.omoba.dev": return "ios.dev.omoba.io"
    default: return nil
    }
}

func omobaAcceptsUniversalLink(_ url: URL, bundleID: String) -> Bool {
    guard let host = omobaUniversalLinkHost(bundleID: bundleID),
          let parts = URLComponents(url: url, resolvingAgainstBaseURL: false) else { return false }
    return parts.scheme == "https" && parts.host == host && parts.port == nil
        && parts.user == nil && parts.password == nil && parts.percentEncodedPath == "/auth"
        && parts.query == nil && parts.fragment == nil
}
