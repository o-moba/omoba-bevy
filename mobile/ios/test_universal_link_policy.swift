import Foundation

@main
struct UniversalLinkPolicyTests {
    static func main() {
        let production = "space.ekza.omoba.beta", development = "space.ekza.omoba.dev"
        assert(omobaAcceptsUniversalLink(URL(string: "https://ios.omoba.ru/auth")!, bundleID: production))
        assert(omobaAcceptsUniversalLink(URL(string: "https://ios.dev.omoba.io/auth")!, bundleID: development))
        for link in [
            "http://ios.omoba.ru/auth", "https://omoba.ru/auth", "https://ios.dev.omoba.io/auth",
            "https://ios.omoba.ru.evil.example/auth", "https://ios.omoba.ru@evil.example/auth",
            "https://user@ios.omoba.ru/auth", "https://ios.omoba.ru:443/auth",
            "https://ios.omoba.ru/auth/", "https://ios.omoba.ru/%61uth",
            "https://ios.omoba.ru/api/auth/vk/callback", "https://ios.omoba.ru/connect",
            "https://ios.omoba.ru/auth?code=secret", "https://ios.omoba.ru/auth?",
            "https://ios.omoba.ru/auth#fragment", "omoba-auth://auth",
        ] {
            assert(!omobaAcceptsUniversalLink(URL(string: link)!, bundleID: production), link)
        }
        assert(!omobaAcceptsUniversalLink(URL(string: "https://ios.omoba.ru/auth")!, bundleID: development))
        assert(!omobaAcceptsUniversalLink(URL(string: "https://ios.omoba.ru/auth")!, bundleID: "unknown.app"))
        print("Universal Link policy: 19 checks passed")
    }
}
