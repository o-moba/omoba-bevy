# TestFlight preparation — 2026-09-16

The owner needs cable-free installation because USB and Xcode Wi-Fi deployment
are unavailable. Added a reusable archive preparer to the primary checkout,
original reproducible beta app icon, API-reason manifest and distribution guide.
No runtime source, server infrastructure, credentials or production dependencies
were changed. The previous development app/home server kit is preserved.

Local verification: iOS Python tooling tests, archive signature and metadata,
resource hashes, opaque 1024px icon and original kit checksums. Input runtime is
the verified September 16 physical-device build; repackaging is not recompilation.

Delivery remains pending: the local keychain has a development identity but no
distribution identity; App Store Connect requires user sign-in. The archive is
development-signed, not an uploaded/Apple-accepted TestFlight build. Account setup,
distribution export, compliance answers, upload processing and phone gameplay
have not been verified. No certificates were created or revoked.

The manifest records file metadata and app interval/timer API reasons; it does
not assert zero data collection or determine encryption export compliance. The
stripped input does not supply dSYMs for complete Rust crash symbolication.
