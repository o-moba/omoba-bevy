# 2026-09-18 — Ekza avatar loop made demo ready

## Goal

Close the remaining gaps between "artist publishes a VRM" and "players see it
in an Omoba match", and make wallet authorization usable from inside the game.

## Changes

- SDK 0.4.1: `passport::pairing::PairingFlow` (non-blocking device pairing),
  `open_in_browser`, `examples/passport_pair`.
- Client: "Connect Ekza wallet" button; status line mirrors pairing progress;
  picker rebuilds when the wallet is approved or the store catalogue arrives.
- `scripts/ekza_publish.py`: chain template -> verified VRM -> Omoba GLB with
  baked clips -> catalogue item, immutable assets, approval record.
- `scripts/ekza_demo.py` and `docs/ekza-avatar-demo.md`: the loop locally on
  devnet.

## Checks

- `ekza_publish.py list` against devnet: 19 templates decoded.
- `publish --index 18` (Robert) reproduced the hand-made roundtrip fixture
  byte for byte: GLB `546ddea7…`, VRM `5e3edaf3…`, hence the same slug.
- `publish --index 14` (Rose), a template nobody had prepared by hand:
  52 bones mapped, 2,629,568-byte Omoba rendition.
- The real registry backend (`ekza-mirror/backend`) loaded the produced
  catalogue and served both templates with their `omoba` approvals.
- `store_e2e` against that registry: both avatars listed, installed, accepted
  by `verify_local`.
- The real passport (`solana-avatars/app`) pointed at that registry listed both
  templates; the SDK `PairingFlow` obtained a live code and approval link, and
  the approval page rendered ("Allow Omoba to read your purchased avatar
  library?").
- SDK 33 tests, Omoba client 378 / server 173 / shared 59 / passport 8.

## Not verified

- The wallet signature and everything after it in a live match (ticket
  consume on the server, second client downloading the model). These need a
  browser wallet that owns an avatar; the code paths are covered by unit tests
  and the byte path by `store_e2e`, not by a played match.
- Mobile builds.

## Remaining risks

- The registry loads its catalogue at start; `serve` must be restarted after
  `publish`.
- Python's HTTP/1.1 client broke on long chunked transfers behind a local
  proxy; the publish tool uses HTTP/1.0 and resumes short bodies.
