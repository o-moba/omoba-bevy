# Game directory and server routing

Status: **concept / TODO**, recorded 2026-10-13 from the owner. Nothing here is
implemented or scheduled. It keeps the idea, the current constraints and a phased
path in one place.

## Idea

Production clients should not decide which game server they talk to. They know one
fixed entry point. A backend behind it routes players to the right lobby and match
servers. Operators then move, add, drain or upgrade servers without shipping a new
client.

## Where it meets the current game

| Area | Today | Why it matters |
| --- | --- | --- |
| Client default | `DEFAULT_GAME_SERVER_ADDR` is a baked **IP and port** (`77.246.105.57:4000`), overridable at build time ([session_config.rs](../../client/src/session_config.rs)) | Moving the server means a new client release |
| Lobby | One coordinator, one region; it redirects to match workers on `OMOBA_MATCH_PUBLIC_HOST` and ports from `OMOBA_MATCH_FIRST_PORT` with `OMOBA_MATCH_CAPACITY` ([match_pool.rs](../../server/src/match_pool.rs), [public MVP](../public-mvp.md)) | The lobby already hands out match servers, but only on its own host |
| Compatibility | Every transport runs a JSON preflight: handshake, release, protocol, catalog, geometry ([release compatibility](../release-compatibility.md), [compatibility.rs](../../shared/src/compatibility.rs)) | Routing can pick a lobby whose contract matches the client instead of failing after connect |
| Web backend | app.omoba.io is a Vercel (Next.js) app: HTTPS only, serverless | It can serve a directory document and an HTTP matchmaking API; UDP game traffic must still go straight to game hosts |
| Contours | Dev builds bake the dev lobby and registry; prod builds will hide the server picker (connection profiles work) | A directory URL per contour can replace hard-coded addresses in both |

## Phases

### D0 — Names instead of IPs (S)

Bake a hostname (for example `play.omoba.io:4000`) instead of the IP. Moving the
lobby then becomes a DNS change. Check that every client path resolves hostnames,
including mobile and party/reconnect. The dev lobby already uses `lobby.dev.omoba.io`.

### D1 — Directory document (S–M)

Clients fetch one HTTPS document at startup:
`https://app.omoba.io/api/game/directory` (dev: `app.dev.omoba.io/...`).

The document carries:
- a schema version, an issue time and an expiry;
- a list of lobbies: `id`, `region`, `host`, `port`, the compatibility contract
  (protocol, catalog, geometry), `status` (open, maintenance, full) and a `weight`;
- a client policy: minimum release, update URL and a message of the day.

The client:
1. picks a lobby whose contract matches its own, then by region or measured
   latency (the existing preflight can time it);
2. keeps the last good document and falls back to a baked hostname when the
   directory is unreachable;
3. never mixes contours: a prod client only accepts the prod directory.

Sign the document with an ed25519 key whose public half is baked into the client,
so a hosting or DNS compromise cannot redirect players. Operators publish it with
a CLI from a reviewed file. Production builds then hard-code only the directory URL
and the fallback name.

### D2 — Rolling upgrades and maintenance (M)

- Route by contract: old clients keep reaching the old lobby until it drains,
  while new clients reach the new one. Upgrades no longer need the
  "deploy the server first" choreography.
- Maintenance mode and "update required" screens come from the document, not from
  a failed handshake.
- Canary: send a share of compatible clients to a new lobby build.

### D3 — Matchmaker and match tickets (L)

- A stateful matchmaker service. It is not serverless; it runs next to the lobbies,
  for example with the Account API on the game VPS. app.omoba.io stays its public
  HTTPS front.
- Flow: the client asks for a match over HTTPS, the matchmaker queues it (across
  regions if needed) and returns a short-lived signed ticket `{worker host:port,
  match id, seat, expiry}`. The worker admits only valid tickets.
- Parties, reconnect to the same worker, bot fill (see the bot personas plan) and
  rating bands move from the single lobby into the matchmaker.

### D4 — Fleet (L–XL)

Several game hosts and regions report health and capacity to the matchmaker.
Optionally autoscale by starting and stopping VPS workers. Add observability:
queue times, fill rate, match start latency.

## Open questions

1. Where the directory lives: a Vercel route in omoba-web, or a static signed file
   on the game host behind Caddy (simpler, no serverless cold start).
2. Regions worth having for the first players. Beijing and Moscow latency differ a lot.
3. Whether the matchmaker in D3 belongs to the Account API or is a separate service.
4. What happens to LAN and self-hosted servers in production builds. Today the plan
   is a desktop-only environment override.

## TODO when scheduled

- [ ] **D0** — hostname default, a resolver test on every platform, DNS record, release note.
- [ ] **D1** — directory schema and signer CLI, client fetch/cache/fallback/selection,
      contour separation, tests (unreachable, expired, bad signature, no compatible
      lobby), a dev directory on the dev contour first.
- [ ] **D2** — contract routing, maintenance and update screens, canary weights,
      upgrade runbook.
- [ ] **D3** — matchmaker service and tickets, worker ticket check, party and
      reconnect, migration away from the direct lobby queue.
- [ ] **D4** — multi-host health, capacity, autoscaling, metrics.
