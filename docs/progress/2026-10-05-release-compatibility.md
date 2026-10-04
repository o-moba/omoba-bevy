# Release compatibility — 2026-10-05

Candidate 0.41.0 introduces a stable pre-game JSON handshake. Network protocol 9, catalogue standard-kits-4 and map geometry remain unchanged; handshake 1 and gameplay revision combat-2026-10-04 establish explicit compatibility contracts. Release labels are informational.

The client verifies every newly created transport before Hello, account bootstrap, Join, commands or snapshots. Confirmed mismatch stops both session and menu automatic retry loops. Timeout remains an unavailable verification, with existing transient-failure retry behavior. Offline practice stays socket-free. Home/loading/search/status screens explain the failure and settings show the compared release labels. Verification is cleared on teardown or endpoint change.

Lobby, worker and standalone dispatch answer bounded non-amplifying requests without allocating players or matches. Public signed-command admission is unchanged. The shared comparison drives the CLI and the release builder's compatibility.json sidecar. The wire-enum inventory separately pins the handshake envelope version.

See [the contract and release procedure](../release-compatibility.md). No CI configuration, secrets, production server or App Store/TestFlight state was changed. The deployed 0.40.1 Beta cannot verify 0.41.0 clients until the new server code is rolled out; existing legacy clients retain the old admission path on the new server.

## Verification and limits

Task evidence lives in `.agent/tasks/RELEASE-COMPAT-20261004/`. The final repository gate passed with 1,558 Rust tests, 39 existing ignored integration tests and 203 Python tests. This includes the menu retry regression found during native-log review; format, strict workspace Clippy and the no-QA client check also pass.

The CLI matrix covered compatible release-label differences, each mismatch category, and a silent UDP endpoint. A compiled local standalone server answered the real CLI probe. An in-memory dispatch test exercised all three server roles without creating players. Native Bevy captured the real home screen at 852×393 in English using an explicitly synthetic incompatible-server report; a repeat improved contrast, and controls stayed inside the viewport. The final retry-only fix does not change the captured layout.

Physical iPhone/Android behavior, real Internet conditions, and a deployed lobby-to-worker round were not tested in this task. Compatibility revisions are maintained contracts, not automatic proof of all simulation behavior or performance. The envelope diagnoses a trusted endpoint; it does not authenticate or encrypt the server connection. Disk space fell below 20 GiB during verification; subsequent checks reused the existing cache. No mobile artifact build or cache deletion was attempted.
