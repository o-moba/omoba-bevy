# Fresh review of current source

- The socket receives no application traffic until a validated correlated report is accepted; mismatches return before Hello or the queued command loop. Unknown/malformed/stale replies are not accepted.
- Client comparisons explicitly reject unsupported handshake versions, including two matching unknown versions. Release numbers do not enter the compatibility decision.
- Server response parsing is capped at 2 KiB, validates nonce and bounded contract identifiers/padding, and enforces response <= request. A process-wide reply budget avoids per-source allocation. All roles run it before gameplay admission.
- Retry, server switches, worker redirects and offline transitions use the same transport constructor, which clears prior contract state. Teardown clears verified metadata. Existing offline local simulation avoids the network thread.
- Both retry owners were reviewed: the joined session loop stops on confirmed rejection, and the menu retry loop now also skips protocol/map/compatibility mismatches while allowing unavailable-server recovery.
- Public proof/signature code is unchanged. Legacy admission remains intentional for server-first rollout; it is not represented as having passed the new handshake.
- The operator CLI uses a manifest produced from the shared Rust contract. The packaging helper rejects a sidecar with a different release and uses a shared target cache. CI and production remain untouched.
- New code, tests, docs and local artifacts remain in the dedicated feature worktree. No credentials or account state are included in evidence. Only a local synthetic mismatch was used for the native screenshot.
