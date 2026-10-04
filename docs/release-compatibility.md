# Client/server release compatibility

Starting with 0.41.0, every new network transport performs a small JSON preflight before sending Hello, identity bootstrap, Join or commands and before decoding gameplay snapshots. Offline practice has no preflight. Lobby → worker redirects, Retry, reconnects and server changes each create a fresh check.

The envelope is independent of the gameplay protocol. The server reports its contract even when the client's gameplay protocol differs. `shared/src/compatibility.rs` is the contract owner; both mobile clients and all server roles use it. The operator CLI uses the same comparison. The contract contains:

| Field | Policy |
| --- | --- |
| handshake | Version of the compatibility envelope/interpretation; retain a parseable envelope when advancing it. |
| release | Informational package version. Different compatible patch releases are allowed. |
| protocol | Bump for incompatible packet/schema meanings in `shared/src/protocol.rs`. |
| catalog | Bump `CATALOG_REVISION` for incompatible equipped skill/catalog contracts. |
| geometry | Bump `GEOMETRY_ID` for incompatible map geometry. Supported runtime map configuration still comes from snapshots. |
| gameplay | Bump `GAMEPLAY_REVISION` for changes to shared simulation semantics/prediction/balance that must ship together, even if the JSON schema is unchanged. |

These revisions are deliberately maintained API contracts, not a claim that a version number proves every behavior. A compatible protocol does not replace gameplay, reconnect or performance tests. Cosmetic UI, textures and animation-only fixes can keep the same contract. Server-only fixes that preserve client expectations can also keep it. Source revision and binary checksum belong in the deployment record separately.

## User-visible behavior

A correlated valid report unlocks normal transport/authentication. A mismatch reports the specific category and stops automatic reconnect; Retry or a different server can run a new check. Settings show game/server release labels and verified/incompatible status. A four-second timeout says verification is unavailable: it does not claim a protocol mismatch or compatibility. Existing bounded reconnect behavior remains available for a disconnected match.

Unanswered/legacy servers never get a silent compatibility bypass. Therefore **deploy the server first**, then distribute 0.41.0 clients. Existing clients retain the old admission path on the new server and are not retroactively certified by this handshake. A pre-0.41.0 server may still be protocol-compatible, but cannot verify the new client and will not admit it through the new client's gate. The currently deployed 0.40.1 Beta must be upgraded before distributing 0.41.0 clients.

## Release commands

Use cache A in the primary checkout (or the coordinator-assigned B):

```sh
export CARGO_TARGET_DIR="$(git rev-parse --path-format=absolute --git-common-dir)/../target"
make -s compatibility-manifest > /tmp/compatibility.json
make compatibility-check GAME_SERVER_ADDR=127.0.0.1:4000 COMPATIBILITY_MANIFEST=/tmp/compatibility.json
```

`make` prints recipes; use `make -s compatibility-manifest` when redirecting stdout to JSON. Alternatively use `cargo run --locked --quiet -p shared --example compatibility -- manifest`. `scripts/release.py build` saves `compatibility.json` beside artifacts, included in checksums. For direct Xcode/Gradle/server builds, generate and retain this manifest from the same frozen source used to build the artifact. Never regenerate an old artifact's manifest from the current checkout.

```sh
# Artifact-to-artifact comparison, no running server needed:
cargo run --locked --quiet -p shared --example compatibility -- compare CLIENT.json SERVER.json
# Live, read-only UDP check. Creates no account, seat or match:
cargo run --locked --quiet -p shared --example compatibility -- check HOST:PORT CLIENT.json
```

Exit codes: 0 verified compatible, 2 confirmed mismatch, 3 unavailable/invalid input. Stdout is JSON, including both full contracts when a valid server answered. Nonzero means **do not distribute this client against that endpoint**; inspect the category instead of retrying release/upload blindly. Check the lobby and a staged worker endpoint; lobby compatibility alone cannot certify that workers were upgraded. These tools do not deploy, change secrets or modify CI configuration.

## Repeatable release sequence

1. Freeze source; decide which compatibility revisions change. Run `make check` (matrix, nonce/size/rate, UDP admission and lifecycle regressions included).
2. Build client/server artifacts and keep their manifests and checksums. Compare manifests.
3. Start the candidate server in staging, probe it with the candidate client manifest, and run the normal signed admission/reconnect/gameplay smoke. Also test a deliberate mismatched manifest: it must exit 2.
4. Under deployment authorization, update lobby and worker binary source together. Probe both roles. Retain prior artifacts and manifests.
5. Publish mobile builds only after the live endpoint verifies compatibility. Record the manifests, probe JSON, source commits and artifact checksums.
6. Before rollback, probe/compare the rollback server with every client version still supported. Do not roll a 0.41 client back onto an unverified 0.40 server. A rollback that changes the contract needs a matching client or another compatible endpoint.

## Bounds and trust

Requests/reports are at most 2 KiB. A 16-byte nonce correlates each attempt, connected UDP filters the peer, server replies cannot exceed request size, and all probe sources share a 64 replies/second budget per process. No per-source compatibility table, accounts, players or match workers are allocated. Existing return-path proof and signed command admission remain required on public servers. The nonce/report does **not** authenticate the server, encrypt traffic, or replace a trusted server address. Deliberately modified legacy clients are outside this accidental-version-mismatch guard.
