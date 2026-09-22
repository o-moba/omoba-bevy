# Test Studio avatars in Omoba on a local iPad

This uses the existing isolated Supabase Studio rehearsal, not production and not Solana. No wallet or purchase is needed for approved free avatars. The existing wallet connection remains optional and opens its own approval page; the local Studio launcher does not launch a wallet storefront or provide payments.

## Prepared local session

- Studio: `http://192.168.1.71:5188/studio`
- Registry: `http://192.168.1.71:8018`
- Game server: `192.168.1.71:4030` (practice 5v5)
- Test avatar: **Robert · OMOBA LAN**
- Existing local creator, reviewer and game-owner credentials: `.ekza-lan/test-accounts.txt`, owner-only, ignored by Git and never served over HTTP.

Keep the Mac and iPad on the same trusted Wi-Fi. Use only these local test accounts over HTTP.

## Install the current iPad game

Open `mobile/ios/Omoba.xcodeproj`, scheme **Omoba**, select the iPad, **Run** (Debug). The build reads `.ekza-lan/client.json` and embeds the chosen Registry origin. Release/Archive ignores this file. The SDK's HTTP LAN allowance only exists in Rust debug builds, is limited to the exact configured private IPv4 host, and never skips authentication, project approval or asset hash/size checks.

In the game select server `192.168.1.71:4030`. In **Avatars**, scroll below Included heroes, tap **Connect Ekza**. Safari opens the Studio confirmation page. Sign in as the local creator, approve the displayed code, return to Omoba. The button can reopen the approval page. The account is saved in private application storage. Refresh the collection after a new avatar approval; library refresh also happens automatically.

The public catalogue lists approved free Studio avatars even before account connection. The account library identifies avatars saved or created by this account. “No approved avatars” is a catalogue state, not a successful login confirmation. Wallet connection status appears separately.

## Upload your own avatar

1. In Studio sign in as the creator and upload a VRM with its correct licence/attribution. Submit it for processing.
2. Sign in as the reviewer; inspect the model and approve publication.
3. As creator, in the avatar’s Games section request `desktop / humanoid-glb-v1` and submit the completed rendition to **Omoba**.
4. As the game owner, open Games → Omoba, inspect the submission and approve it.
5. Save the avatar to the creator/player library. Refresh Avatars in the game, select the Studio avatar and equip it before starting the match.

A successful new upload alone does not approve a game version. The worker must produce a valid rendition and the game owner must accept it. A rejected or failed build remains unavailable to the game; inspect its Studio report. The validated Robert sample is retained in the local test catalogue.

## Restart the local services

Dependencies are the repositories' existing pinned requirements: Registry `uv sync --frozen` from backend, Studio `npm run build` from web. The existing local Supabase project must be running. The launcher never resets/migrates it or creates/rotates accounts. It refuses a rehearsal file that references a remote Supabase.

Build the current debug game server with `cargo build --locked -p server --bin server`, then from the canonical Omoba checkout:

```sh
python3 scripts/ekza_lan.py --host 192.168.1.71 \
  --registry-repo ../ekza-registry \
  --rehearsal-state ../ekza-registry/.build/cross-app-20260922/private.json \
  --game-binary target/debug/server
```

Use `--game-binary` for an existing cache's current server executable. The default ports are 8018/5188/4030. Occupied ports fail clearly and existing services are not killed. Ctrl+C stops only the launcher’s own process groups; Supabase data and local settings remain. To return Debug builds to the public Registry, rename/remove only `.ekza-lan/client.json` and rebuild. Do not use this local HTTP configuration for production accounts or published builds.

## Verification boundary

The real local cycle uploaded and published a VRM, built and approved its Omoba rendition, paired an account, fetched its library through the SDK, verified downloaded bytes with Omoba's validator, and admitted that avatar to a live ten-player practice snapshot. Native desktop rendering and iOS compile/link checks supplement regression tests; they are not a claim of physical iPad Safari/swipe testing.

The public wallet endpoint returned HTTP 503 during this session (2026-09-23). Its operational availability is separate from native browser handoff. The game now shows its pairing state/error; this task did not deploy or repair that production service. Local Studio account authorization and free-avatar use were tested successfully.
