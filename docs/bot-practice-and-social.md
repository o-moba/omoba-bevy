# Bot practice, match chat and reactions

The standalone practice launcher starts a playable arena with one human and fills
vacant seats with labelled bot heroes. The separate public lobby offers Quick bot
fallback and Play with bots, saves approved human progression, and freezes its
allocated roster. See [the public multiplayer guide](public-mvp.md). The behavior
below describes standalone practice unless explicitly stated otherwise. Chat and picture reactions work in admitted matches and
use a separate, bounded social stream.

## Start a practice session

From the repository root:

```sh
make practice
```

This builds the locked native sources, starts one practice server and client,
and prints the session log directory. Choose a hero and Join. The default is
5v5: one human and nine server-controlled heroes. Closing the client or pressing
Ctrl+C stops the processes owned by this launcher. To use another local port:

```sh
make practice LOCAL_SERVER_ADDR=127.0.0.1:4010
```

To keep a practice host open for other testers, use separate terminals:

```sh
make practice-server LOCAL_SERVER_ADDR=127.0.0.1:4010
make game GAME_SERVER_ADDR=127.0.0.1:4010
```

For LAN testing, bind the host to `0.0.0.0:4010` and give clients its reachable
LAN IP address. `0.0.0.0` is a bind address, not a client destination. The host
remains running until Ctrl+C. `make game2d GAME_SERVER_ADDR=127.0.0.1:4010`
selects the 2D renderer for another client.

The equivalent server setting is `OMOBA_MATCH_MODE=practice`; server-only hosts
may also set `OMOBA_TEAM_SIZE`. The supervised `make practice` launcher uses
five seats per team. Existing `make play` and `make play-bots` keep the legacy
release-session launcher with external harness players. They have not been
renamed or changed into the new practice mode. Use Practice for the built-in
server bots; see [career setup](match-progression.md) for authenticated Release.

## Bot and result behavior

Bots use eight existing free 3D avatar models matched to their classes and retain
their independent 2D sprite kits. They use ordinary hero navigation, attacks and skills on the actual map. They
push lanes, engage nearby enemies and structures, retreat at low health, and
can die and respawn. The controller uses the existing movement, targeting,
damage, cooldown and mana rules. Bot actor IDs are marked explicitly and their
internal addresses reject network commands. They do not send chat or reactions.

In standalone practice, late humans receive their own player identity and safe
spawn; the server removes
a bot on the assigned team rather than transferring its score or inventory.
Reconnect keeps the human's retained identity. Full human capacity rejects a
further join. A round's lifetime participant list is bounded to 32 identities;
repeated arrivals/refills that would exceed it trigger a fresh practice round
instead of reusing historical identities. Connected humans stay for that reset.

Standalone practice results are local, unsaved match receipts. They give no permanent XP,
MMR, profile counters or PostgreSQL match history, even when `OMOBA_DATABASE_URL`
is configured. The account/friends service may still be available independently.
Practice is not a ranked shortcut. The persistence adapter also rejects any
bot-marked participant in a rated allocation or result. Publicly allocated bot
games instead use the approved `public-casual-v1` policy: durable human history and
50/25 win/loss XP, unchanged Elo, and no new human joining a running roster.

## Chat and reaction controls

- Press **Enter** or the visible **Chat** button to open chat. Enter or **Send**
  submits the draft; **Escape** or **Close** exits. The channel button switches
  between **Team** and **Match**. Unicode text and IME input are supported.
- Press **T** or **Reactions** to open the four-choice wheel. Click a picture or
  use **1–4**; Escape cancels.
- On touch, hold the local living hero for about **450 ms**, then drag to a
  picture and release. Moving more than 12 scaled pixels before the wheel opens
  cancels the hold. Releasing outside a choice or receiving an OS touch-cancel
  sends nothing. HUD controls and an existing movement/attack gesture take
  precedence. Death, loss of focus or a viewport change cancels the wheel.

Opening chat/the wheel blocks gameplay input and cancels existing orders.
The closing frame remains blocked so a release cannot also move or attack.
Reactions appear over the actual sender in both render modes for up to 2.8 seconds;
server age and event IDs prevent duplicate snapshots from restarting them.
Reactions require a living hero during Running. Chat is accepted during Running
and Victory. Mute controls are local display preferences, not server moderation.

These controls passed scripted native desktop and phone-preview capture/input QA.
Physical iOS/Android gesture acceptance is not established by a desktop phone-sized window.

## Server validation and transport

Chat is a trimmed single line with at most 160 Unicode characters and 640 UTF-8
bytes. Empty text, control characters and directional override/isolation controls
are rejected. The server resolves player ID, nickname and team from its admitted
player/account; a request cannot choose its sender identity. Team messages are
filtered before serialization and never sent to the opposing team.

Chat and reactions share three tokens, refilling one token every two seconds.
The limiter follows the account or guest session across endpoint replacement and
round changes. Requests carry epoch, match, session and monotonic request IDs;
duplicates do not emit another event. Authenticated account actions use the
existing signatures. Unsigned requests cannot advance an authenticated sender's
acknowledgement sequence. Guest development traffic uses the existing admitted
session boundary; the UDP transport is not encrypted.

The server retains at most **32 events for 30 seconds** and bounds sender/limiter
maps to 512 entries. Social data is ephemeral and does not enter PostgreSQL or
match receipts. The client keeps at most 40 received chat entries for the current
round. A scoped `Subscribe` command opts into social snapshots without consuming
a chat token or emitting an event. Current clients subscribe after joining or
changing rounds; legacy peers receive no unsolicited social packets. Subscribed
clients receive a separate framed `Social` packet at most every 250 ms (4 Hz),
with its own fragment namespace. Old round/epoch data is discarded.

## Author a reaction catalog and its pictures

[shared/assets/reactions.json](../shared/assets/reactions.json) is the versioned
server/client catalog, embedded at build time. It defines stable reaction IDs,
labels, pack IDs and access policies. Edit it and rebuild matching client/server
packages to add a reaction; it is not a remote catalog or a hot server override.
IDs contain lowercase ASCII letters, digits, `_` or `-`, with at most 64 bytes.
The catalog permits at most 64 packs and 256 reactions in 64 KiB. Unknown fields,
duplicates and missing pack references fail validation. Keep the four free base
IDs: `thumbs_up`, `thumbs_down`, `heart`, `laugh`. An invalid packaged catalog,
or one that removes/locks a base reaction, falls back to the independent base
catalog.

The client presentation file is `client/assets/reactions/manifest.json`, loaded
as `reactions/manifest.json` beneath its asset root. It maps IDs to packaged PNG
images; it grants no access. For example, a complete four-choice manifest can
use one atlas and a separately authored heart image:

```json
{
  "schema_version": 1,
  "wheel": ["thumbs_up", "thumbs_down", "heart", "laugh"],
  "images": {
    "thumbs_up": {"path": "reactions/base-atlas.png", "grid": [2, 2], "index": 0},
    "thumbs_down": {"path": "reactions/base-atlas.png", "grid": [2, 2], "index": 1},
    "heart": {"path": "reactions/your-heart.png", "grid": [1, 1], "index": 0},
    "laugh": {"path": "reactions/base-atlas.png", "grid": [2, 2], "index": 3}
  }
}
```

Supply `your-heart.png` yourself with suitable redistribution permission; this
example does not ship that file. Grid indices run left to right, then top to
bottom. Each grid dimension is 1–16; the index must fit. Image dimensions must be
nonzero, at most 8192 per side, and divisible by the grid. The wheel has exactly
four distinct IDs with image entries. The manifest is bounded to 64 KiB and 256
images. Paths must be relative packaged `.png` paths: no URLs, absolute paths,
drive prefixes or traversal. Missing/invalid manifest data uses the built-in
base presentation. A custom local image or wheel entry never bypasses the
server's `allowed_reactions` list. Ship art and its notices with the package.

## Entitlements and the NFT boundary

The **current runtime is free-only**: `server/src/social.rs::social_sender`
constructs `Entitlements::default()`. Protected policies are validated and
covered by policy tests, but no paid grant source is wired into that sender.
Editing a client file, choosing an avatar or including a mint/URL in a packet
cannot unlock a pack.

The catalog's access variants are:

| `access` | Meaning |
| --- | --- |
| `{"kind":"free"}` | Available to every admitted player. |
| `{"kind":"operator_grant"}` | Requires an explicit trusted operator pack grant; not evidence of NFT ownership. |
| `{"kind":"verified_avatar","avatar_id":"…"}` | Companion pack deliberately associated with one approved canonical avatar. |
| `{"kind":"nft","provider":"ekza_passport"}` | Reserved future provider policy; always denied in this version. |

`Entitlements` has no wire deserializer. Its `grant_operator_pack` method accepts
bounded trusted configuration. `grant_verified_avatar(expected, consumed)` checks
the exact approved avatar/rendition response; it must only be called **after**
successful one-use consumption by the operator's trusted Passport service for
the current game session. That local method alone does not verify the network
origin, expiry, current token holder or session.

A future adapter must obtain the trusted proof outside the gameplay tick, bind
the resulting grant to the admitted account/session and intended pack, define
expiry/revocation, clear it on reconnect/round changes, and pass the resulting
entitlements into `social_sender`. The existing avatar Passport only covers its
approved Solana-devnet avatar contract. It does not verify arbitrary sticker NFTs,
and the career device key is not currently linked to a wallet. Generic NFT packs
need a new trusted provider/service contract and an explicit authorization path;
the reserved `nft` policy cannot be unlocked with an operator or avatar grant.

This feature adds no production dependency, marketplace, minting, wallet storage,
NFT ownership service, global chat, party chat or voice. See the
[avatar Passport integration](progress/2026-09-11-avatar-passport-roundtrip.md)
for the narrower existing purchased-avatar path.

## Verification

Tests cover shared text/catalog/entitlement policy, bot lifecycle and ordinary
combat, server audience filtering/rate limits, actual UDP framing and client
gesture/input state. The final run commands, results and native captures belong
to `.agent/tasks/BOTS-SOCIAL-2026-09-14/evidence.md` and `evidence.json`.
Integrated tests and nine native captures passed. The captures use real server
echoes and scripted input, with OS focus confirmation and measured UI bounds.
No physical-device or external NFT-service proof is claimed here.
