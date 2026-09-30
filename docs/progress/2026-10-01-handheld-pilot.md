# Handheld pilot — 1 October 2026

Warrior now defaults to an original Blender sword. The shared cosmetic catalog
also includes a hammer and scepter, selectable for any class in Combat Test.
The existing Ekza SDK imports approved free static props using
`omoba / desktop / handheld-glb-v1`; built-in and imported items use the same
semantic VRM hand attachment. Equipment is replicated to local and remote actors.
See [contract and commands](../handheld-weapons.md).

## Native evidence

Evidence directory (repository-relative, local QA artifacts):
`.agent/tasks/HANDHELD-PILOT-2026-09-30/verified-swap/`.
Base commit: `7fb78af9c6dbf051b689c5cb556e75a22c66d3b9`, with this uncommitted pilot.
Captured client SHA-256:
`adcd49122a1f3cad1b19434ad365ef665ddb5bf1826859765f782890dddfd100`.
Subsequent edits only remove an unused import and add a regression test/docs.

One English desktop viewport, 1280×720, Warrior, supported camera zoom 55%.
The real `weapon-build` binary validates/exports the scepter; a loopback HTTP
fixture exposes an explicitly approved free catalog entry; `weapon-import`
downloads via the pinned SDK and verifies SHA-256/size. The HTTP fixture stops
before the real server and client start. `sdk-proof.json` records the catalog
query, download, installed item and native result. This is not a public Space
listing or a live ownership check.

| Capture | Checked result |
| --- | --- |
| `native/06-idle.png` | Agnes with the default sword; separate enemy sword |
| `native/07-running.png` | Run animation; item follows the animated hand |
| `native/02-sword-attack.png` | Server accepts basic attack; sword visible in swing |
| `native/03-hammer-attack.png` | Hammer replaces sword on the same avatar |
| `native/04-scepter-avatar-swap.png` | Actual `avatars/orion.glb` rig with SDK-imported scepter |
| `native/05-empty-hands.png` | Orion empty-handed; enemy keeps its own sword |

All six gameplay captures were visually inspected. Assertions confirm loaded
scenes, correct owner IDs, the actual avatar model and animation binding, Idle/
Run/Attack states, four accepted basic-attack sequences, and exact hand-parent
transform agreement (maximum recorded position error: 0.0). Close finger curl
and every possible swing angle are not certified by these frames. The common
one-handed clip remains a placeholder for weapon-specific animation polish.

The first visual pass exposed an existing sandbox race: between snapshots the
legacy fallback could recreate the previous model and move the camera to a team
spawn. Avatar replacement now occurs before applying the same incoming snapshot;
a regression test covers replacement without a fallback gap. Earlier `first/`
and `framing/` captures are superseded and are not evidence of a successful
two-avatar transfer.

## Automated checks

- Rust socket calculation covers both hands of all 15 shipped free VRM rigs.
- Catalog/GLB validation, real SDK HTTP download, hash rejection and server
  acceptance of cosmetic choices across all 16 classes are tested.
- Three GLBs pass Assimp inspection and the candidate-asset policy audit.
- Local Registry source/rendition profile checks accept all three GLBs;
  the builder rejects invalid input with exit 2 and structured issues.
- Full `make check`: PASS (format, workspace and no-QA Clippy, Rust workspace
  tests, Python 152, iOS tooling 44). Client: 820 passed / 1 existing ignored;
  passport: 28; server: 297 / 3 existing ignored; shared: 103; common: 87.
  Existing database/integration skips remain; no physical-device test was run.
  Log: `/tmp/handheld-verified-check.log`.

## Remaining rollout

1. Register the supplied profile and `omoba-handheld-v1` builder; publish and
   approve props through the public Space/Registry creator flow.
2. Add production equipment UI and ownership tickets/checkout for paid items.
3. Add finger grip posing, weapon-specific clips, two-handed IK, bows and strings.
4. After this pilot, choose defaults for other classes and verify mobile packaging
   and performance on physical devices.

No public registry, SDK source repository, production infrastructure or purchase
flow was changed. The pilot's importer accepts only approved **free** props.
