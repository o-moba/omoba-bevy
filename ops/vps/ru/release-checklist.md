# RU release checklist

Run this for every RU release before it is announced, and keep the result: copy the
[result template](#result-template) to `docs/progress/<date>-ru-release-<version>.md`
and fill it in. A release ships only when every row is PASS or an owner-approved N/A.
This replaces the April 2026 gameplay-slice gate
([docs/release-gate-checklist.md](../../../docs/release-gate-checklist.md), kept as a
historical record).

Platforms: Windows, Linux, macOS and Android (iOS has no RU build,
[docs/markets.md](../../../docs/markets.md)). Use the packages from
`dist/v<version>-ru/` exactly as published (the same sha256 as `SHA256SUMS`), never a
local debug build.

`<RU portal>`, `<RU Account API>` and `<RU lobby>` (host:port) below are the RU
profile's names in [scripts/markets.py](../../../scripts/markets.py), the only place
they are written; `python3 ops/vps/ru/ru_host.py names` prints them.

## 1. Server and build gate (once per release)

| # | Check | How | Pass when |
| --- | --- | --- | --- |
| S1 | Local gate | `make check` on the release commit (the owner decides on local results, not GitHub CI) | green |
| S2 | RU packages | `make release-ru`; keep `compatibility.json` and `SHA256SUMS` | all four platforms built |
| S3 | Server ship | `ru_host.py ship --package dist/v<v>-ru/omoba-<v>-ru-linux-x64.tar.gz --check`, then `--apply` | preflight `ok`, result `DEPLOYED` (or `UNCHANGED`), the pre-migration backup listed (an off-host set and a rollback dump) when a schema existed |
| S4 | Portal ship | `ru_host.py portal-bundle --web <omoba-web>`, `ru_host.py ship-portal --bundle … --apply` | `DEPLOYED` (or `UNCHANGED`) |
| S5 | Host status | `ru_host.py status` | `"ok": true`, no problems (backups included: a set off-host from the last 26 h, a passed restore drill, a current escrow) |
| S6 | Public preflight | `make release-ru-preflight` from a computer outside the host | prints `"ready": true` |
| S7 | Lobby compatibility | `cargo run --locked --quiet -p shared --example compatibility -- check <RU lobby> dist/v<v>-ru/compatibility.json` | exit 0 |
| S8 | Worker compatibility | while a bot match from P3 runs: the same command against `<RU IPv4>:41000` (or the worker port in `journalctl -u omoba-lobby-ru`) | exit 0 |
| S9 | Reboot (new host, or after a kit change) | `ru_host.py reboot-check --apply` | `"ok": true`; note `seconds_to_ready` |
| S10 | Downloads | Publish the GitHub release `v<v>-ru` (its tag is the source link in the game and the README; `catalog` refuses until it exists), then `make release-ru-publish` (RU host + IPFS mirror), `releases.json` installed ([README › Downloads](README.md#downloads)), then `make release-ru-verify-downloads` | `"ready": true`: every platform listed on `<RU portal>/downloads/`, status «Открытая бета», each package and its mirror match the catalog's sha256 and size |
| S11 | Download from a Russian network | Download one package from `<RU portal>`'s download page over a Russian connection (no VPN); `ru_host.py downloads --since <today>` | the sha256 matches the page and the count for that platform went up; note the network and platform |

## 2. Per platform

Run each row on every platform with a fresh player unless the row says otherwise.
For Android use a Vulkan device from the published minimum list (LR-25).

| # | Check | Steps | Pass when |
| --- | --- | --- | --- |
| P1 | Clean install, first run in Russian | Remove the `omoba-bevy-ru` settings folder (Android: uninstall `space.ekza.omoba.ru`). Install the package, start it. Open both documents from the card, tick the switch, press «Продолжить». Restart the game. Open Settings → «Об игре». | The legal card (LR-06) with `12+` comes first, in Russian, with no English or raw keys; the documents open on `<RU portal>`; the lobby is not reached before «Продолжить» and is reached after it; after the restart the card does not come back; About shows the build, `12+`, both documents and the source of `v<v>-ru`; no crash |
| P2 | VK sign-in | Profile → «Войти через VK ID» → sign in on `<RU portal>` → confirm the link in the game. Then sign in on the portal in another browser. | The game shows the linked account; the portal shows the same profile; a second device can join the account through VK |
| P3 | Bot match | Play a server bot match («Боты на сервере») to the end. | The match ends normally; the result and XP show in the game and in the history on `<RU portal>` |
| P4 | Casual queue | Two clients (any platforms) queue Casual. | They land in one match (bots fill after the wait); the match is unrated and appears in both histories |
| P5 | Ranked queue | Queue Ranked. | The searching screen shows the queue count; after 5 minutes it offers Casual without leaving Ranked (O-1); with 10 humans the match is rated |
| P6 | Reconnect | Mid-match, kill the client (Android: swipe it away), start it again within 180 s. | «Вернуться в матч» rejoins the same seat; a bot played it meanwhile |
| P7 | Update over the previous version | Install the previous RU release, sign in, play one match, then install this release over it. | Same profile, device key and history; settings kept; no second account |
| P8 | Incompatible server | Start the previous release (or a client with a different protocol) against the new server. | The localized update-required prompt with a download button (LR-14), no endless reconnect, no crash |

## 3. Decision

- Any FAIL in S1–S8, S10–S11 or P1–P8 blocks the announcement. Fix and rerun the affected
  rows, or roll back (`ru_host.py rollback`, [README › Rollback](README.md#rollback)).
- Record N/A only with the reason and the owner's approval.
- The owner signs off the result file in `docs/progress/`.

## Result template

```markdown
# RU release <version> (<date>)

Commit <sha>; packages dist/v<version>-ru/ (SHA256SUMS sha256 <…>); server release
<version>-ru-<sha7>; portal release <rev7>-<sha8>; tester <name>.

| # | Result | Evidence / notes |
| --- | --- | --- |
| S1 | PASS | `make check` <time> |
| S2 | | |
| S3 | | |
| S4 | | |
| S5 | | |
| S6 | | |
| S7 | | |
| S8 | | |
| S9 | | seconds_to_ready = |
| S10 | | |
| S11 | | network, platform, count |

| # | Windows | Linux | macOS | Android (device, Android version) |
| --- | --- | --- | --- | --- |
| P1 | | | | |
| P2 | | | | |
| P3 | | | | |
| P4 | | | | |
| P5 | | | | |
| P6 | | | | |
| P7 | | | | |
| P8 | | | | |

Blockers: <none | list>
Decision: <ship | no ship>, <owner>, <date>
```
