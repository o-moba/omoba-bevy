# Open Moba

**A shared world for avatars. Built in the open, together.**

Open Moba is an open-source multiplayer MOBA built with Rust and Bevy. It is a
place to play, compete with friends and give creator-made characters a life
inside a game. It is also a practical integration of the **Ekza SDK**: we want
other game developers to build on the same ideas and connect their own worlds.

An avatar drawn by an artist, modeled by another person and brought to life by
an animator should have a future beyond one application. We are working toward
that future through a real game, reusable tools and open collaboration. Bring
a character, a map, a sound, an idea or a pull request. Help shape what comes next.

[Open Moba website](https://omoba.io/) ·
[Ekza ecosystem](https://ekza.io/) ·
[Ekza Space](https://space.ekza.io/) ·
[Ekza Bevy SDK](https://github.com/ekza-space/ekza-bevy-sdk) ·
[Our mission](MISSION.md) · [Contribute](CONTRIBUTING.md)

## Choose your way in

| I want to… | Start here |
| --- | --- |
| Play a match with bots | [Run locally](#play-locally) with `make practice` |
| Test on an iPhone | [Device build](mobile/ios/README.md) and [TestFlight workflow](mobile/ios/TESTFLIGHT.md) |
| Make characters, worlds or effects | [Create with us](#create-with-us) |
| Bring avatars into another game | [Integrate Ekza](#bring-ekza-into-your-game) |
| Host matches or connect a phone and PC | [Hosting and connection](#host-a-practice-match) and [runbook](RUNBOOK.md) |

## What you can play today

This is a **native beta under active development**. The source version is
[`0.21.0-rc.1`](Cargo.toml); see [features](docs/features.md) and
[changes](CHANGELOG.md) for the detailed implementation history. A source version
is not a promise of a published installer or a live public server.

- **Team combat:** four classes, basic attacks and class abilities, lane minions,
  towers, jungle camps that respawn, items and match results.
- **Bot practice:** native server bots fill empty seats; people can join and take
  over those seats. Practice is separate from ranked progression.
- **Desktop and phone controls:** mouse/keyboard on desktop; movement joystick,
  right-thumb attacks, directional targeting and skill inspection on phones.
  The compiled platform selects the interface; both use the same game protocol.
- **A persistent career:** PostgreSQL-backed profiles, `nickname#1234` handles,
  match statistics, history and friends on configured career servers. A companion
  player portal reads the same data through the
  [Account API](account-api/README.md).
- **A world to customize:** character presentation, projectiles, effects, reusable
  map props, tower placement, balance data, music and sound.
- **Cosmetic Supporter work:** shared device accounts, recovery codes and three
  server-authorized auras. Payment providers are disabled until configured;
  Apple verification integration and real payment validation remain unfinished.
  See [current Supporter status](docs/supporter.md).

The built-in roster and local practice need **no wallet, purchased avatar or
PostgreSQL database**. Mobile build tools live in this repository; device tests,
store distribution and a reliable public service remain separate release work.

## Play locally

Install [Rust](https://rustup.rs/), Git, Python 3.9+ and Make. The commands below
use a POSIX shell. Your OS also needs the native build and graphics libraries
required by Bevy; see [Bevy's Linux dependencies](https://github.com/bevyengine/bevy/blob/v0.18.0/docs/linux_dependencies.md)
when building on Linux.

```sh
git clone https://github.com/o-moba/omoba-bevy.git
cd omoba-bevy
make help
make practice
```

`make practice` builds the locked current sources, starts a local 3D server with
native bots and opens the client. Choose your hero and join. The first build can
take several minutes; subsequent runs reuse Cargo's cache. Close the client or
press **Ctrl+C** to stop only the processes owned by this launcher.

Logs are saved in this checkout's `target/local-play/sessions/`. If port 4000
is occupied, use `make practice LOCAL_SERVER_ADDR=127.0.0.1:4010`.
`CARGO_TARGET_DIR` can relocate compiled binaries; it does not relocate these
session logs. No adjacent project checkout is needed.

To exercise full-roster matchmaking instead, `make play` (alias `make play-bots`)
launches a release-match-mode server and nine external harness bots. This is a
different workflow from native bot practice. Match mode and Cargo optimization
profile are separate settings.

## Host a practice match

On the computer hosting the game:

```sh
make practice-server LOCAL_SERVER_ADDR=0.0.0.0:4000
```

On another computer, replace the example address with the host's actual LAN IP:

```sh
make game GAME_SERVER_ADDR=192.168.1.10:4000
```

On the phone, enter that same reachable `host:4000` in the game's connection
screen. `127.0.0.1` on a phone means the phone itself. Both devices must be able
to reach the host's UDP port. This command runs a foreground practice server;
24/7 public hosting additionally needs supervised operations and network setup.

## Play with a friend (party vs bots)

Host a practice server as above, then both of you start the client against it
(`make game GAME_SERVER_ADDR=<host-lan-ip>:4000`; the host can use
`127.0.0.1:4000`). No database or wallet is needed.

Players with a downloaded build ([GitHub Releases](https://github.com/o-moba/omoba-bevy/releases))
set the host address in the game: **Party & friends → SERVER → Change**, type
`host:port`, Enter. It is remembered.

1. On Home, open **Party & friends**. Everyone connected to the same server is
   listed under *Online on this server*; press **Invite**.
2. Your friend sees *"… invites you to a party"* on Home (or in the party
   screen) and presses **Accept**. Both avatars now stand in the party line-up.
3. The leader presses **PLAY VS BOTS** (or **PLAY AS PARTY** on Home). Every
   member moves to hero select; lock in your heroes.
4. The whole party is seated on one team and bots take every other seat
   (two friends: you + 3 bots against 5 bots). The draft waits up to a minute
   for party members who are still picking.

On a public lobby (`OMOBA_SERVER_ROLE=lobby`) the same party queues together:
Play with bots allocates one arena with the party on one team, and Quick match
never splits a party. Details: [docs/party.md](docs/party.md).

Persistent profiles and match history require a configured career server and
PostgreSQL. See [career setup](docs/match-progression.md),
[Account API operations](account-api/README.md) and [RUNBOOK.md](RUNBOOK.md).

## Build release packages

`make release-check` shows what this computer can build; `make release-ci`
builds macOS, Windows, Linux and Android on GitHub Actions into a draft
release; `make release-testflight` uploads the iPhone build. See
[docs/RELEASING.md](docs/RELEASING.md).

## Commands at a glance

Run **`make` or `make help`** to see commands without starting a game or build.

| Command | Purpose |
| --- | --- |
| `make practice` | Local client + native bot server; easiest first match |
| `make practice-server` | Native bot host; set `LOCAL_SERVER_ADDR` for LAN access |
| `make play` / `make play-bots` | Full 5v5 matchmaking demo with external harness bots |
| `make game` / `make game2d` | One client, in 3D / orthographic 2D; set `GAME_SERVER_ADDR` |
| `make server` | Full-roster matchmaking by default; solo players wait for a filled queue |
| `make server-dev` | Explicit development mode; first join starts the match |
| `make start` / `make start-release` | Legacy server + client workflows; details in the runbook |
| `make bots BOTS=4 BOTS_SERVER=127.0.0.1:4000` | Add external harness players to an existing server |
| `make iphone-check` | Check Xcode and physical iPhone build prerequisites |
| `make iphone` | Build an **unsigned** device package; signing is a separate step |
| `make verify-gameplay` / `make verify-task-12` | Headless gameplay/matchmaking checks and live UDP QA |
| `make stop` / `make restart` | Broad legacy local-process cleanup / restart; not session-scoped |

For iPhone, install Xcode and the Rust `aarch64-apple-ios` target first. Follow
[signing and installation](mobile/ios/README.md) or the
[TestFlight archive guide](mobile/ios/TESTFLIGHT.md). `make iphone` does not upload
to Apple. Retain `builds/` when clearing disposable `target/` caches. Android and
iOS Simulator workflows are in the [mobile guide](mobile/README.md), with their
measured limits.

## Bring Ekza into your game

**Open Moba is a working reference, and an invitation to build another game.**
Ekza's broader goal is an ecosystem where creators collaborate on avatars and
assets, and participating games can recognize compatible representations of
them. Each game keeps its own art direction, performance budget and rules.

| Project | What to explore |
| --- | --- |
| [Ekza Bevy SDK](https://github.com/ekza-space/ekza-bevy-sdk) | Rust character identity, model metadata, GLB validation and Bevy model loading |
| [The SDK revision used by Open Moba](https://github.com/ekza-space/ekza-bevy-sdk/tree/28fbfde54780ba6af469a1653ef9334df2f5f446) | The exact integration, including Passport data contracts; SDK `main` may differ |
| [Ekza Stellar TypeScript SDK](https://github.com/ekza-space/ekza-stellar-sdk) | Asset manifests, avatar loading and application bridge source |
| [Ekza Stellar creator protocol](https://github.com/ekza-space/solana-stellar) | Collaborative assets, lineage, releases and contributor-share infrastructure |
| [Ekza Space](https://space.ekza.io/) | Explore another application in the ecosystem |

For a Bevy integration, start with the SDK's README and model-checking examples.
Then follow Open Moba's [pinned dependency](client/Cargo.toml),
[native Passport adapter and importer](passport/src), and
[staged avatar integration guide](docs/progress/2026-09-11-avatar-passport-roundtrip.md).
The guide includes reproducible commands and the division of responsibility
between client, trusted Passport service and authoritative game server.

The current purchased-avatar path is **opt-in, staged Solana Devnet desktop
integration**. A trusted service verifies ownership, and the game approves the
exact model and animation profile before use. It does not make every model
playable in every game, hot-load unknown avatars during a match or establish
mobile Passport readiness. A compatible file format alone does not establish
permission to redistribute the artwork.

**SDK reuse status:** the Bevy SDK revision pinned above has no declared license
grant yet, as recorded in the [licensing audit](docs/progress/2026-09-12-open-source-licensing.md).
That needs to be resolved for third-party adoption; Open Moba's licenses do not
license a separate SDK. The TypeScript link points to source, not a verified npm
package installation workflow.

## Create with us

You can help build this world without writing Rust. We welcome artists,
designers, modelers, riggers, animators, musicians, translators, players and
programmers. Start small, show what you made and credit the people who helped.

| Create or improve… | Guide |
| --- | --- |
| Characters, weapon/projectile skins and combat effects | [Combat cosmetics](docs/combat-cosmetics.md) |
| Props, forests, rivers, towers and map composition | [Map customization](docs/map-customization.md) |
| Combat pacing, camps, distances and balance | [Balance tuning](docs/balance-tuning.md) |
| Music, interface sounds and combat cues | [Audio authoring and sources](docs/game-audio.md) |
| Playability, accessibility and phone usability | [Playtest checklist](docs/playtest-script.md) and [bug report](docs/bug-report-template.md) |
| Shared tools, integrations or documentation | [Contribution guide](CONTRIBUTING.md) and [issues](https://github.com/o-moba/omoba-bevy/issues) |

We want collaboration to leave people with more possibilities: useful tools,
shared knowledge, fair credit and worlds they can help shape. Contributors
retain their copyright; permissions and asset provenance remain explicit.
Read [our mission](MISSION.md) and the [contribution terms](CONTRIBUTING.md).

## Controls

| Desktop | Action |
| --- | --- |
| Right-click ground / minimap | Move using map navigation |
| Left-click hostile | Select a target without moving or casting |
| Right-click hostile | Approach and repeat basic attacks |
| Q / W / E / R | Use class abilities |
| Tab / S or Backspace | Select nearest enemy / stop and clear target |
| P / Shop | Buy equipment while alive at your base |
| Y / Space | Toggle camera follow / return to your hero |
| Mouse wheel / Alt + right mouse | Zoom / orbit in 3D |
| Esc | Close a panel or open the pause menu |

On phones, use the left movement joystick and right-thumb **ATTACK** button with
abilities around it. Drag ATTACK to choose a target along a direction; the short
handle does not extend attack range. Hold an ability still to inspect it, tap to
cast, or drag deliberately to aim. See [platform controls](mobile/README.md) and
[practice, chat and reactions](docs/bot-practice-and-social.md).

## Technical and project references

The Bevy client and authoritative Rust server communicate over UDP; the website
uses a separate HTTP Account API. Snapshots and career replies use bounded,
1,200-byte datagram framing with complete-message reassembly. Player account
identifiers remain separate from display handles and avatar ownership proofs.

- [Feature inventory](docs/features.md), [changelog](CHANGELOG.md) and [runbook](RUNBOOK.md).
- [Account API](account-api/README.md), [career storage](docs/match-progression.md) and [player handles](docs/player-handles.md).
- [Supporter implementation and rollout limits](docs/supporter.md).
- [Historical beta package guide](docs/progress/2026-09-07-beta-test-guide.md) and [its dated readiness record](docs/progress/2026-09-07-beta-readiness.md).

## Licenses

Server source: **AGPL-3.0-only**. Client and reusable original source:
**MPL-2.0**. Original documentation and identified Verdant art:
**CC-BY-4.0**. Existing CC0/OFL and other dependency/asset terms remain unchanged.
Commercial forks are welcome under the applicable terms.

Read the [license map](LICENSING.md), [brand policy](TRADEMARKS.md),
[attribution](ATTRIBUTION.md) and [source distribution guide](SOURCE.md). The
mission expresses the official project's direction; it adds no restrictions to
those licenses. Avatars, SDKs and third-party assets retain their own terms.
