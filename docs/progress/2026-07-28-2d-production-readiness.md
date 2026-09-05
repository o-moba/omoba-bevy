# 2D production-readiness pass — 2026-07-28

Task: `TASK-2D-PRODUCTION-READINESS-01`

## Outcome

The reported invisible towers and minions had two independent causes. Their
occupied art was projected to roughly 3.6 px and 0.95 px respectively at the
maximum supported zoom-out, and snapshots slightly above 8 KiB were truncated
by the client receive buffer before JSON decoding. The render-only scale and
camera pass now keeps lane towers near 14.7 px and minions above 6.1 px on
their occupied major axis at maximum zoom-out. No gameplay radius, map anchor,
AI, combat value, or server-authoritative transform changed.

All six lane towers derive their Top/Mid/Bot identity from the authoritative
lane polylines at the existing team samples (Green 0.30, Blue 0.70). Each has
one primary proxy, one team-shape badge, and one lane label. Every lane minion
has one primary proxy and one team-shape badge. Green uses a square and Blue a
diamond so the teams do not depend on hue alone. Reconciliation runs after
snapshot/interpolation updates and recursively cleans bounded child cues when
an owner disappears.

The character-selection portrait strip was expanded from five to ten cells by
deterministically appending the repository-local character-pack portraits. The
original five decoded RGBA cells remain byte-identical. The Orchard portrait
is a point-resized derivative of its approved repository-local static master;
it is selection art only and is not represented as gameplay animation.

## UDP contract

- One JSON `ServerPacket::Snapshot` remains one UDP datagram.
- Client and harness receive storage is 65,536 bytes.
- A legal IPv4 UDP application payload is at most 65,507 bytes.
- The server serializes and validates the complete payload before `send_to`.
- Client requests keep the 8,192-byte request policy, but receive storage is
  large enough to identify and reject an oversized request whole.
- Malformed/decode/send diagnostics are rate limited; partial JSON is never
  published as a snapshot.

A real release-mode 5v5 regression receives one complete runtime-dependent
snapshot satisfying `8192 < bytes <= 65507`, containing 10 players, 8
structures, and 18 minions, then observes valid snapshots after malformed and
8,193-byte requests. On this macOS host, a separate loopback probe reports
`EMSGSIZE` above 9,216 bytes. That lower kernel send ceiling is not buffer
truncation; future snapshot growth beyond it needs payload reduction or a
separately versioned fragmentation/compression design.

## Deterministic portrait assembly

No external or generative art call was made during this task. Existing local
CC0 outputs were assembled with ImageMagick 7.1.2-22:

```sh
magick master-approved-safe.png -filter point -resize 256x256 orchard.png
magick original-five.png cathedral-256.png crab-256.png giraffe-256.png \
  ram-256.png orchard.png +append -define png:color-type=6 portraits.png
```

Final atlas: `client/assets/presentation2d/portraits.png`, 2560×256 RGBA,
SHA-256 `80d73b35be392f6b6e28a96e1d5d6b95f6c975a4efc0d083d4d726cdcf2373c2`.

## Verification

- `cargo build --workspace` — PASS.
- `cargo test --workspace --no-fail-fast` — PASS (including 94 client, 55
  server, real UDP, matchmaking, gameplay, and sprite-cosmetic tests).
- Focused presentation ECS tests — PASS (11/11).
- Focused client network tests — PASS (13/13).
- Real-server UDP datagram regression — PASS (1/1,
  `8192 < runtime-dependent bytes <= 65507`).
- Presentation atlas contract, including occupied bounds and ten portraits —
  PASS.

Final full lint/asset/proof-loop results are stored under
`.agent/tasks/TASK-2D-PRODUCTION-READINESS-01/`.

## Remaining release blocker

`orchard-comet-centaur` still has no repository-local runtime locomotion or
action sheets. Its approved static master is not substituted for six-state
animation, so Orchard remains intentionally non-PASS until the six source
clips, 48 selected frames, two runtime sheets, and provenance are completed.
The task makes no native GPU visual claim because this environment reports no
usable GPU.
