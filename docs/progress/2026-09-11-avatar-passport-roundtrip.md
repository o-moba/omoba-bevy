# Purchased avatar passport in Omoba

Version: 0.19.0-rc.1. Date: 2026-09-11.

The native client now pairs to the browser wallet, lists owned Omoba-supported
avatars, requests a scoped ticket, and enters through authoritative server
verification. Games consume the free SDK; ownership controls the purchased
cosmetic, and existing class selection and gameplay remain independent.

## Reproduce the staged integration

Cargo pins the published SDK revision
`8254ed5e94d4709c11c83bef604ebe4481467847`. A clean checkout needs no sibling SDK
directory or local Cargo patch. The lockfile records this same revision.

For the verified paid Robert fixture, first start the `demo/avatar-roundtrip`
stack from the `solana-avatars` repository. It serves the approved catalog and
the exact rendition bytes; use its Passport API URL below. Building the game
and playing with the free roster do not require this service or wallet pairing.

```sh
cargo build --locked -p server -p client -p omoba-passport
export OMOBA_PASSPORT_URL=http://127.0.0.1:5190/api/passport
cargo run -p omoba-passport --bin passport-import -- /absolute/path/approved-roster.json
```

Open the printed verification URL in the wallet browser and approve its public
code. The importer reads the resulting owned library, downloads approved Omoba
renditions without auth headers, bounds bytes, verifies SHA-256 and size, checks
embedded humanoid skinning and the idle/walk/attack/cast/death clips, and writes
the public manifest atomically. It preserves the shipped free roster. Approval
is an explicit registry decision for `omoba/desktop/humanoid-glb-v1`; GLB format
alone does not approve compatibility.

For that fixture the importer recreates
`client/assets/avatars/ekza-dbd08c9e5440ce0e45556e44468cdbe037e61c8d9a712d1d4ad4167db75d4868.glb`.
This generated copy is deliberately ignored by Git. Run the import before
starting the purchased-avatar demo on a fresh checkout.

Distribute the generated GLBs under `client/assets/avatars/` and the same public
manifest to each client and server. Set the following separately for the two
processes, using an absolute manifest path:

```sh
OMOBA_AVATAR_MANIFEST=/absolute/path/approved-roster.json SERVER_ADDR=127.0.0.1:4018 target/debug/server
OMOBA_AVATAR_MANIFEST=/absolute/path/approved-roster.json GAME_SERVER_ADDR=127.0.0.1:4018 OMOBA_PASSPORT_CONNECT=1 target/debug/client
```

Pair the client using its own newly printed code. Select the purchased entry in
the ordinary avatar picker, choose a class/team, and join. Without
`OMOBA_PASSPORT_CONNECT=1` the game opens with the free roster. With no supported
purchases, pairing reports an empty library and the free roster stays usable.
Custom packages can set `OMOBA_ASSET_DIR` to their distributed asset root.

Only a one-use ticket enters the game packet. The server uses its own trusted
`OMOBA_PASSPORT_URL`, exact project/session scope and approved rendition. Missing,
expired, replayed, rejected or mismatched proofs fail closed. HTTP verification
runs outside the gameplay tick with at most 16 concurrent requests. New joins
and reconnects obtain fresh tickets; ordinary UDP retries reuse the current one.
Guests cannot reclaim retained paid appearances through a free avatar request.

## Evidence and limits

The local frozen contract and per-criterion results live in
`.agent/tasks/AVATAR-ROUNDTRIP-20260911/`; this verification archive and generated
assets are kept outside source control. The standalone command below tests real
UDP admission, second-client replication and forged/rejected paid joins against
a deterministic local HTTP fixture; it is not a chain or GPU proof.

```sh
cargo test --locked -p omoba-passport -p shared
cargo test --locked -p server passport
python3 scripts/check_passport_admission.py --server target/debug/server --output /tmp/omoba-passport-check
```

Real local testing uses the paid devnet template
`solana:devnet:avatar-data:3bfPehBVoBKXUUzmGGVT3tgQTYkpispqUKfPW1UktASL`
and Robert rendition SHA-256
`546ddea729486de56289aef537a16bc716c7085dd93f31c66fd9ab3d740783c2`.
Its original VRM source SHA is
`5e3edaf330577ee4c3f6440b8989af3722e7c800bb90eb037f1c05cdfe61fd7c`;
mesh/skin/material data are identical, with animation clips appended. Detailed
runtime captures distinguish the admitted hero from five labeled render-only
creature fixtures. Automated captures do not certify manual interaction or all
combat animations.

Observed on 2026-09-11: native pairing returned the actual buyer's one supported
purchase, the authoritative game server consumed its ticket and admitted the
protected Robert slug, and five Bevy GPU readbacks completed in 59.8 seconds.
Each recorded 11/11 scenes ready and one playing animation node. The river and
orbit gameplay images were visually inspected. Final tests passed: SDK 10,
client 191, server 78, native transport/importer 7, shared 29, and nine real UDP
fixture checks. The independent review's mixed-platform importer issue was
fixed and reverified. Full logs and acceptance limits are in the task evidence.

After pinning the published SDK revision above, the locked client/server/importer
build passed without a sibling SDK override. Verification also passed SDK 10,
native transport/importer 7, shared 29, server admission 3, and nine real UDP
fixture checks, including regeneration of the imported model and sidecar.

This initial path stages assets and restarts both processes. It does not hot
download unknown avatars during a match. The importer creates one approved
owner's roster per run; operators can curate public protected entries for a
larger distribution. Tokens are process-local and wallet pairing is repeated
after restart. Public Solana RPC limits or a passport outage can deny a paid
join; the shipped free roster remains available. No production deployment,
wallet key storage, infrastructure change or new external library was made.

Work is isolated in a clean local clone on `feat/avatar-passport-roundtrip`
because the original repository's linked-worktree metadata was root-owned.
The original cinematic working tree is unchanged.
