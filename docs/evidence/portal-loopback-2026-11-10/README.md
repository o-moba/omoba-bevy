# Current RU portal loopback checks — 2026-11-10

## Result and exact scope

PASS for the two current portal integration gates, with no production deployment or
live account data:

- game code `5cd10d83f4b1016060830b58f5b763e19e3f96ea`;
- portal `ff8f50165bdce761425e27369d6fff7d8fd250ea`;
- RU production Next standalone build (`OMOBA_BUILT_MARKET=ru (ekza off, supporter on, vk_sign_in on)`,
  `OMOBA_BUILT_CONTOUR=production`);
- real RU-market Rust Account API and loopback PostgreSQL fixtures;
- Node 24.22.1, rustc 1.95.0, headless Chrome.

| Gate | Result | Report |
| --- | --- | --- |
| Signed game-directory HTTP delivery | PASS, 7 checks | [directory-report.json](directory-report.json) |
| Leaderboard, privacy and seasonal placement | PASS, 5 check groups | [leaderboard-report.json](leaderboard-report.json) |
| Portal unit tests | PASS, 257; 3 expected skips | terminal result on the same portal tree |
| Portal typecheck | PASS | terminal result on the same portal tree |

The directory run proves exact signed-byte proxying without browser account headers,
native offline verification, anonymous HEAD, POST 405, foreign-region 400, corrupt
publication 503 and atomic replacement without restarting either service. Its report
has `unexpected_requests=[]` and `private_fixture_removed=true`.

The leaderboard run proves ordering and the caller's own row, no MMR/raw codes in the
page copy, no horizontal overflow at 390 px, settings opt-out and its explanation, and
the current-season placement line. The second synthetic profile visibly rendered
`Калибровка 3/20`; the test no longer waits for the obsolete pre-season `Новичок` copy.

## Strengthened leaderboard follow-up

The conditional calibration check is removed. The test now requires at least two
entries with different ratings, verifies the complete API tie-break and matching DOM
order, and requires the second fixture to have lifetime experience but fewer than 20
current-season games. A fresh isolated loopback run passed with ratings 1351/900 and
mandatory visible `Калибровка 1/20`; the report records the observed text and ratings.
Portal unit tests again passed 257 with 3 expected skips, and typecheck passed.
[Machine-readable follow-up report](leaderboard-strengthened-report.json)

The browser run used the existing RU Account API and previous seed binary, then an
explicit disposable SQL fixture after projection and the registration trigger. The
edited three-key seed moves synthetic counters before genuine settlement to avoid that
race. Its targeted `cargo check` passed before this reorder; current formatting passes,
while the committed SHA still needs CI and a fresh executable run under the local disk
guard. No production portal code changed. S9 policy was corrected separately; no RU
reboot or production acceptance is claimed.

## Fixture repairs bound to these runs

`account-api/examples/seed-local-portal.rs` now gives its synthetic profiles the
existing ten-public-completion maturity fixture before using the real Ranked allocation
and settlement path. Without it, current admission correctly rejected the outdated
fixture with `ranked_locked`.

`tests/leaderboard.e2e.mjs` now reads `profile.ranked.rated_matches` before the
lifetime fallback and accepts the current seasonal placement copy. These are test
fixture repairs, not changes to production admission, settlement or presentation.

Source SHA-256:

- `account-api/examples/seed-local-portal.rs`:
  `4d246590ef98b08525c2bf34ec004f43b31fee254c36f1e7f13f31866f6214e4`;
- `tests/game-directory.e2e.mjs`:
  `93e49a5f9392cce187b15d67c035f4656ba5a9bee60ca680e1f2f428c8f95db3`;
- `tests/leaderboard.e2e.mjs`:
  `8b1e8ad0fc64e64f5ef5f8647a02e7f36c8210146c3a78ab2fad68817400ba4d`.

Binary SHA-256:

- Account API: `b44e44ade776d698724be72753aa2a7abe1c3afe8ee82f0ef8ead31fcaa3c323`;
- portal seed fixture: `330ea72580c00356dd84989d17d2c08040d4c1510231124300232404faf0a9ed`;
- directory signer: `dea4d28fa44674a1140e4b716364ec399f17b7c2027d84ee21e8340682e6797d`.

The lightweight Account API/fixture build used Cargo cache B and `OMOBA_MARKET=ru`.
An earlier global-market API correctly returned directory 503 and is not accepted as
RU evidence. The final API was rebuilt for RU before both passing integration runs.
All owned API/Next processes were stopped in `finally`; the PostgreSQL containers and
their non-production fixture data remain local.

## Limits

These are loopback integration results only. They do not prove physical-device input,
native key storage, organic matchmaking, settlement at scale, WAN behavior, an isolated
RU host, live signing-key provisioning, client routing/failover, public deployment or
production account data. They do not change the original 62 done / 5 partial / 1 open
finding count or the RU **NO SHIP** decision.
