# Ekza avatar loop: demo runbook

Shows one avatar travelling the whole way: an artist publishes a VRM, an
operator approves it for Omoba, a player buys it, wears it in a match, and a
second player sees it. Devnet chain, local services; nothing is deployed.

## One-time setup

| Need | Command |
| --- | --- |
| Registry environment | `cd ../ekza-mirror/backend && uv sync` |
| Storefront build | `cd ../solana-avatars/app && npm install && npm run build` |
| A devnet wallet in the browser (Phantom/Solflare on **devnet**) with a little test SOL | `solana airdrop 1 <address> --url devnet` |
| `PINATA_JWT` in `../solana-avatars/app/.env` | only for step 1 (uploading a new VRM) |

## The demo

```bash
# 0. what is already on chain
python3 scripts/ekza_publish.py list
```

1. **Artist publishes** (skip to reuse an existing template): open
   `http://127.0.0.1:5191/deployer`, upload a VRM and a preview, set name,
   supply and price, sign. Note the new template index from `list`.

2. **Operator approves for Omoba**:

   ```bash
   python3 scripts/ekza_demo.py publish --index 18 --reviewed-by "Your name"
   ```

   This downloads and verifies the source VRM, bakes `idle/walk/attack/cast/death`
   into a `desktop/humanoid-glb-v1` GLB, and writes
   `.ekza-demo/registry/{catalog.json,assets/,approvals/}`. The run is
   deterministic: the same template always yields the same files and slug.

3. **Start the services** (restart after every `publish`, the registry loads
   its catalogue at start):

   ```bash
   python3 scripts/ekza_demo.py serve
   ```

   Registry `:8029`, storefront + passport `:5191`, game server `udp :4028`.

4. **Player buys**: `http://127.0.0.1:5191/minter`, pick the avatar, mint with
   the devnet wallet.

5. **Player wears it**:

   ```bash
   python3 scripts/ekza_demo.py client --player 1
   ```

   In the picker press **Connect Ekza wallet**. The browser opens the approval
   page; confirm the code and sign with the wallet that owns the avatar. The
   picker rebuilds and the avatar appears under **Your Ekza avatars**. Select
   it, pick a team. On join the client installs the verified model, asks the
   passport for a one-use ticket, and the server consumes it.

6. **Second player sees it**:

   ```bash
   python3 scripts/ekza_demo.py client --player 2
   ```

   Player 2 has its own empty store (`.ekza-demo/player-2`). When player 1
   comes into view, the client looks the slug up in the public catalogue,
   downloads and verifies the model, and swaps it in; until then the legacy
   model stands in.

## What to say while showing it

- The game never talks to Solana. Ownership is checked by the passport against
  the chain; the game speaks HTTP through `ekza-bevy-sdk`.
- The wire id `ekza-<sha256>` is the hash of the avatar identity and the exact
  file hash. The server recomputes it from the consumed ticket, so it needs no
  avatar files and cannot be talked into a skin the wallet does not own.
- Every downloaded model is checked for size, SHA-256, GLB envelope and the
  clips Omoba's animation graph needs, before it is placed or loaded.

## Troubleshooting

| Symptom | Cause |
| --- | --- |
| "Your Ekza avatars" stays empty after connecting | wallet does not own an avatar approved for `omoba`; check `http://127.0.0.1:5191/api/passport/catalog` lists it with `projectId: omoba` |
| `Passport service could not be reached` | `serve` is not running, or `OMOBA_PASSPORT_URL` points elsewhere |
| join rejected with "avatar not authorized" | ticket expired (60 s) or the game server uses a different passport origin than the client |
| `publish` fails on download | public gateways are flaky; rerun, downloads resume |

## Going to production

The same pieces, deployed: serve the published catalogue and assets from
`registry.ekza.io`, run the storefront with the passport routes as one
long-lived Node process, set `OMOBA_PASSPORT_URL` on game servers. The game
and the SDK need no change; their defaults already point at those origins.
