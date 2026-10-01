# Hosted asset lifecycle: OMOBA 0.32.0

Avatar and weapon publication share Studio revision/rendition/project-review primitives. OMOBA keeps separate validators and selectors: avatars use `humanoid-glb-v1`; static free weapons use `handheld-glb-v1`. `/v2/avatars` remains the legacy avatar route, while the new typed SDK queries `/v2/assets?kind=weapon&project=omoba&platform=desktop&profile=handheld-glb-v1`.

## Runtime ownership

- `passport::weapon_store` owns asynchronous catalogue membership, status, verified downloads and per-user cache state. `ekza://weapons/<slug>.glb` uses the already-mounted writable Ekza source. The SDK enforces canonical typed identity, exact approval, GLB format, size and SHA; OMOBA validates the hashed attachment contract.
- Collection → Weapons and draft equipment choices address immutable IDs. Choice persists in client preferences and reconnect loadouts. Join, draft and player snapshots carry only cosmetic selection; clients cannot supply URLs or approval flags.
- `server::passport_admission` reads its own typed catalogue before admitting hosted equipment, then independently authorizes the avatar. Async completions re-enter the remaining gate. Draft results remain subject to epoch/match/generation/request identity. Reclaim checks the requested equipment and reconciles only that cosmetic selection; choosing empty hands cannot restore a revoked old prop and does not reset authoritative combat state.
- Approved entries are trusted for at most five minutes before the next selection refresh. An unknown item can request another read after ten seconds. A healthy empty result removes eligibility; an outage preserves previously known approvals, matching existing avatar behavior. Existing matches/files are not remotely erased.
- Rendering resolves a verified ready definition and attaches it through semantic VRM hand sockets. Other clients trigger the same lazy installation from replicated IDs. Collection and draft show per-selection download/ready/retry status. Draft/loading miniatures use the accepted replicated equipment and the same attachment path; the party lobby shows the local player’s current equipment (remote party presence does not yet advertise equipment). Missing/incompatible bones show no prop. Built-in and historical operator imports retain their local pilot path; production server admission trusts only shipped props and its own live approvals.
- Humanoid Collection previews use the same shared motion and skin-index binding as matches. Extra authored clips are retained when remappable to the validated rig. Preview QA checks advancing playback plus actual joint rotation rather than treating clip names as proof of animation.

## Verification boundary

Focused checks and native evidence are recorded in `.agent/tasks/EKZA-ASSET-LIFECYCLE-20261001/` by the coordinator. Do not interpret this implementation note as hosted or phone acceptance until that evidence is PASS. Paid equipment, two-handed IK, physical mobile certification and IPFS migration are outside this change.

Native hosted harness accepts `OMOBA_SDK_PACK_QA_WEAPON=<approved-sdk-item-id>` and otherwise keeps the shipped sword. It records exact downloaded model paths, actual bone motion, authoritative match admission, running distance and attachment error. The lifecycle coordinator verifies creator/reviewer actions, withdrawal/restoration and a second independently rendered client.

## Two-client native proof hook

Run the same SDK-pack QA on two native clients against one ordinary server, each with a separate `OMOBA_CLIENT_CONFIG_DIR`, `OMOBA_SDK_PACK_QA_OUTPUT` and session. Both receive the hosted one-entry avatar manifest, `OMOBA_SDK_PACK_QA_WEAPON=<approved weapon slug>` and `OMOBA_SDK_PACK_QA_REMOTE_WEAPON=<same slug>`. The second may use `OMOBA_SDK_PACK_QA_MATCH_ONLY=1`; both mount their own initially empty cache. Each waits for the other's replicated equipment, verified hosted model paths, loaded prop, advancing animation, skeletal motion, movement and matching hand attachment. They capture `22-remote-equipped.png` and leave an overlap period before exit. `verify_native(..., weapon=slug, remote_weapon=slug)` validates the added evidence. `OMOBA_AUTOJOIN_WEAPON` is also available for ordinary non-QA observer launches (`default`, `empty` or item ID); it never bypasses server admission.
