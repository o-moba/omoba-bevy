# iOS return from VK ID sign-in

Prepared production Universal Link: **`https://ios.omoba.ru/auth`**.
It is not active until the domain association is published and a newly signed
app is installed. The existing public website and installed apps are unchanged.

| Contour | App identifier | Associated domain |
| --- | --- | --- |
| Production | `QSRX2Y2RJB.space.ekza.omoba.beta` | `applinks:ios.omoba.ru` |
| Development | `QSRX2Y2RJB.space.ekza.omoba.dev` | `applinks:ios.dev.omoba.io` |

The App ID prefix was read from existing OMOBA provisioning profiles. The normal
iOS app supports an RU account region; this change does not invent a separate RU
App Store record or package. The dev app cannot claim the production link.

## Behavior

The native app still starts an expiring device enrollment and opens the portal's
`/connect?code=...`. VK ID returns to the existing web callback. After the player
confirms on the website, an iPhone/iPad can follow the dedicated return link.
The separate subdomain matters: Safari keeps a same-domain Universal Link in the
browser. A server-rendered fallback explains manual return when the app is absent
or the OS has not associated the domain yet.

The main nib attaches `OmobaLinkDelegate` to `UIApplication`. This is winit 0.30's
supported custom-delegate mechanism. It adds no window or scene, swizzling or
replacement event loop. Rust references the Swift archive member before
`UIApplicationMain` so its Objective-C class survives static linking. Xcode and
the CLI packagers compile the same nib.

Only HTTPS, the current app's exact host and `/auth` are accepted. Credentials,
ports, fragments, query parameters and foreign paths are rejected. The handler
does not parse VK tokens, approve requests or switch accounts. Foregrounding
resumes the existing signed enrollment poll and its in-game confirmation.
A cold start only opens the game, it does not restore an in-memory enrollment.

This is a return-to-game link for the existing web flow, not a new VK iOS SDK
integration. The VK web callback must stay
`https://omoba.ru/api/auth/vk/callback`. An iOS platform setting alone does not
enable native SDK authorization.

## Activation gates

1. Publish the companion omoba-web association endpoint over HTTPS, without a
   redirect, at `https://ios.omoba.ru/.well-known/apple-app-site-association`.
   See that repository's `docs/ios-universal-links.md` for the narrow host setup.
2. Enable Associated Domains for the existing Apple App ID and refresh its
   provisioning profile. The locally inspected profiles have no such capability.
   No credential, certificate or Apple account was changed by this implementation.
3. Build/install from the new source using the appropriate Xcode scheme. The
   `CODE_SIGN_ENTITLEMENTS` setting selects one domain for its contour. CLI signing
   refuses an old profile instead of silently dropping the capability. Archive
   preparation retains the source app's validated domains when re-signing.
   A prebuilt binary from before this change cannot gain the delegate by repackaging.
4. On a physical iPhone, install after publication, tap the link from Notes or a
   different website, then test a real VK enrollment, return and in-game approval.
   Check an unlinked VK identity, an expired request and a cold start separately.
   Apple's association CDN may need time to fetch the new file.

DNS, TLS, website deployment, Apple profile refresh, full native app compilation
and physical-device acceptance remain explicit rollout steps. No release or
successful live VK/device test is claimed by the source checks.

## Local checks

```sh
python3 -B -m unittest discover -s mobile/ios -p 'test_*.py' -v
xcrun swiftc mobile/ios/UniversalLinkPolicy.swift mobile/ios/test_universal_link_policy.swift -o /tmp/omoba-link-policy
/tmp/omoba-link-policy
```

Also compile `UniversalLinks.swift` with the iPhoneOS SDK and inspect both
Xcode schemes' `CODE_SIGN_ENTITLEMENTS`. The bridge and nib can be checked without
a full Rust rebuild or touching signing credentials.

References: [Apple associated domains](https://developer.apple.com/documentation/xcode/supporting-associated-domains),
[Apple Universal Links](https://developer.apple.com/documentation/xcode/allowing-apps-and-websites-to-link-to-your-content).
