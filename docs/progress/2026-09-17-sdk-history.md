# SDK history cleanup and consumer pin — 2026-09-17

The SDK's Claude coauthor trailer was removed, and the sole human owner's Git
identity normalized to the requested gitwotori alias. SDK source trees and commit
timestamps were preserved. Both public branches retain their original topology.

Open Moba's previous Passport pin `8254ed5e94d4709c11c83bef604ebe4481467847` maps to `28fbfde54780ba6af469a1653ef9334df2f5f446` with the exact same Git
tree. The four consumer manifests, Cargo.lock source ID and README link are updated.
No dependency is added, no version is upgraded, and desktop/Android features stay
unchanged. Historical progress notes retain their original SHA as historical data.

The SDK is used by character identities, Passport data contracts and validation.
Current bundled-roster GLB loading and purchased-avatar delivery implementation
remain in the game/Passport wrapper; the legacy SDK remote catalog is not loaded
at startup. See docs/features.md for the integration boundary.

Backups and the SHA map are outside caches under
`_workspace/backups/ekza-bevy-sdk-git-2026-09-17` in the umbrella workspace.
