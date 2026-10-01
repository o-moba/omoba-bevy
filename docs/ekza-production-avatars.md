# Publish the SDK Twenty collection through real Ekza Studio

This is an operator/creator runbook, not a deployment or publication performed by the local collection task. The local catalog is only a consumer-path proof. Production Studio must assign real avatar IDs and manage revisions, processing and game approval; do not upload the developer `catalog.json` as the production catalog.

## Observed public service state

Read-only checks on 2026-10-01 against `https://registry.ekza.io` returned:

| Endpoint | HTTP |
| --- | --- |
| `/healthz` | 200 |
| `/v1/studio/status` | 404 |
| `/v1/profiles` | 404 |
| `/v2/avatars?project=omoba&platform=desktop&profile=humanoid-glb-v1` | 404 |
| `/studio` | 404 |

The existing registry is reachable, but these checks do not establish a deployed account-first Studio service. A separate public Studio frontend origin has not been verified. The routes below are relative to the operator's deployed Studio origin, not a claim that `/studio` exists on the API origin today.

## 1. Operator: make the real pipeline available

Use the Registry release procedure in the sibling repository's `docs/stabilization-release.md`. This requires a separately approved deployment, including its database migration and configuration steps.

- Release compatible Registry API, Studio frontend, Supabase migrations/Auth/private Storage, and the trusted worker. Public upload ingress must accept representative VRMs, not only small test requests.
- Configure the isolated processor image and read-only builder assets. Set the Omoba rendition builder to `scripts/ekza_build_rendition.py` from the intended Omoba revision. Do not use the single-model cached rehearsal converter for this twenty-model batch.
- The current global revision-processing path also builds/validates USDZ before curator publication. It therefore needs a working Blender/USDZ processor even when the eventual consumer is Omoba. Omoba itself loads GLB on phones; the USDZ file is for Mirror.
- Verify Studio readiness, profiles, catalog, uploads and public HTTPS asset downloads. Set the API's Studio web origin so device-account approval links point to the deployed frontend.
- Provision a curator and assign the intended reviewer account as project owner for `omoba`. Creator registration alone does not grant either privilege. The operator CLI supports `project-owner omoba <account-uuid>`.

## 2. Creator: prepare the account and source records

Sign up/sign in at the deployed Studio's `/studio?view=account`, using the intended Open Source Avatars publishing account. Complete email confirmation if required by that deployment.

The public `creator` is the publishing account. Original authors belong in the avatar's **Attribution** field; uploading from a collection account does not transfer that creator field to the original artist.

Use [selection.json](../assets-src/sdk-avatar-pack/selection.json) for the exact 20 names, original paths, source URLs and pinned hashes. The archive root is `/Users/wotori/Yandex.Disk.localized/models/opensourceavatars`. Preserve original source bytes.

The selected collection records credit **Polygonal-Mind**. The R2 VRMs credit Polygonal Mind directly; R3 embedded author fields often contain a deployer/wallet identifier, which is not an independently verified list of individual artists. Retain those records, check any separately supplied artist credits, and do not invent individual names. Some external R3 `otherLicenseUrl` documents were inaccessible during selection and remain unverified.

Suggested attribution for each card, adjusted to verified source credits:

```text
Original collection credit: Polygonal Mind — 100Avatars R3.
Source: <that avatar's original source URL>
Published in this collection by Open Source Avatars.
Individual artist: <include only when verified from the source>.
```

The local selection's collection and embedded metadata declare CC0; copy the actual source terms to **Usage license** and resolve any contradictory source-specific terms before publishing. Attribution should retain original creator/source information even for CC0 works.

## 3. Creator: publish one pilot, then the other nineteen

Start with EYEWizard to exercise the whole live path before repeating it for the batch.

1. Open `/studio?view=new`. Enter the original name, description, usage license and attribution.
2. Upload that entry's original `model.vrm`, not its locally generated GLB. Source limit is 50 MiB.
3. Optionally upload a real PNG cover, at most 5 MiB and 4096×4096. Many R3 files named `thumbnail.png` actually contain JPEG bytes: convert them to PNG or omit the cover; renaming is insufficient.
4. Choose **Upload & submit**. Wait for processing and inspect failures instead of treating upload success as publication.
5. A curator opens **Review queue**, inspects the exact revision, then chooses **Approve & publish**.
6. In the avatar's **Games** panel, choose **Check requirements → Prepare for Omoba**. Wait until the rendition is ready, then **Submit to Omoba**.
7. The Omoba project owner opens the game's submission queue, inspects the built file, and chooses **Approve for the game**.

Global publication and game approval are separate gates. The resulting production `ekza:avatar:…` identity/SDK slug will differ from the local developer identity; preserve the original source ID as provenance instead of trying to reuse the developer slug.

## 4. Operator/player: verify the mobile path

Current `passport::selector()` always uses `SupportSelector::omoba_desktop()`: iOS and Android use the same `omoba / desktop / humanoid-glb-v1` selector as the desktop client. The word `desktop` is a current contract name, not a reason to submit the Mirror `ios / arkit-body-v1` rendition to Omoba. Separate mobile optimization/budgets can be introduced later with coordinated profiles and clients.

- Confirm the approved model appears in the public v2 catalog with `access: free`, matching Omoba approval, SHA-256, size and a reachable HTTPS GLB URL.
- Use an Omoba game server and mobile client pointed at the same public registry. The SDK default is `https://registry.ekza.io`. Remove local `OMOBA_REGISTRY_URL` overrides and LAN rehearsal settings; an old development build may retain a compiled local origin. Xcode Archive deliberately strips LAN settings. Build/install a current production-configured client when necessary.
- On the phone open **Avatars → Studio**, refresh, preview and equip the model, then join a real match. A second client should see the same avatar. Check animation, camera-facing direction, scale, sword attachment, load time and memory on iPhone and Android before approving the rest for release.
- Approved free catalog entries can be discovered without signing into the publisher account. **Save to my library** and **Connect Ekza** are for a player's own saved collection/device link; players must not share the Open Source Avatars publishing credentials. No wallet or NFT mint is needed for this free Studio route.
- Confirm both cold download and cached relaunch. The server independently checks catalog admission; new approvals/withdrawals may take its existing five-minute TTL to appear. A fresh server avoids that TTL during the pilot. Already downloaded files are not remotely erased.

Once this live pilot passes, repeat publication/preparation/approval for the remaining nineteen. Subsequent compatible catalog additions do not require embedding models into each app release; the SDK downloads them on demand.

## Current proof boundary

The local twenty-model task verified rendition construction, content hashes, actual SDK cold installs and authoritative server admission for all twenty. Native visual verification stopped because the Mac was locked. Neither public Studio publication nor iPhone/Android visual/performance acceptance was completed by that task.
