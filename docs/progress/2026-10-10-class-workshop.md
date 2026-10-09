# Community class workshop — 2026-10-10

The first creator catalogue iteration is extended into a working class workshop
across the game/Account API, player portal and marketing site. The user explicitly
requested this prototype now; it supersedes the older roadmap's proposed delay
of web class authoring until later artist gates.

## Delivered

- Existing `/creators` covers 17 engine classes, 68 skills and 148 requirements.
  `/workshop` composes the 12 modular cores and 48 reusable skills. The twenty
  legacy class-owned abilities remain browse-only. Core equipment and borrowed
  skill references remain separate.
- `shared::workshop::ClassBuildDocument` supplies bounded strict import,
  authoritative recipe validation and safe native preset conversion. Capability
  rules are shared with the resolver rather than copied into a web balance table.
  The launcher accepts `--class-build` and isolates local test configuration.
- Existing OMOBA Account API/PostgreSQL stores owner-private class preferences,
  with quota 50, optimistic versions, transactional idempotency and current consent.
  Migrations 004/005 and minimal runtime grants are source changes only; no hosted
  database was touched.
- An explicit Ekza library device flow binds an encrypted project-scoped grant to
  the OMOBA profile. Browser responses contain display state, not credentials.
  A username is not treated as an immutable identity, and no SSO merge is claimed.
- Marketing points from the configured creator portal origin to both gallery and
  workshop. No undeployed production URL, upload approval or automatic skin
  installation is advertised.

See [community walkthrough](../community-workshop.md),
[game document/import contract](../class-workshop-game.md),
[Account API contract](../../account-api/docs/openapi.json) and the companion
[portal PR](https://github.com/o-moba/omoba-web/pull/1).

## Source and compatibility

Exported gameplay source is `f5fb8c1` on this PR. The workshop content revision is
`sha256-34e41fefa3d9795e4278635ac6a2a4f7f8f3a4ce6ed7247477e639cfc2c517ed`;
asset catalogue revision is
`sha256-23e35f191223f5dfc63a30b7c99754e6b2fc36046bc5ce7d830cc8cbe5227c23`.
Both include source provenance and consumer integrity checks. The workshop's
source commit follows consumed source paths, so committing its generated snapshot
or unrelated documentation does not make `--check` stale.

Build document schema 1, gameplay catalogue `standard-kits-6`, protocol 11 and
`combat-2026-10-08-mechanics` are retained. Public/ranked custom recipe admission
is unchanged. Match host validation remains authoritative.

## Verification

- Shared 134 tests, asset exporter 18, launcher 5 and source-provenance regression 1
  passed. Snapshot check passed after the separate generated-snapshot commit.
- Account API 29 tests passed, including all real PostgreSQL integration suites:
  signed pairing, owner separation/injection, strict recipes, version conflicts,
  idempotency, concurrent quota, revoked sessions and restricted-role grants.
  Ekza tests cover wrong scope/project, encrypted owner binding, outage retention,
  expired/revoked grants and response whitelisting.
- The existing Account API and synthetic Registry protocol fixture ran against a
  dedicated local PostgreSQL 18 cluster at 55581. The demo runtime role is not the
  database owner. Hosted accounts, production secrets and infrastructure were not
  used or changed.
- An actual browser download, “Radiant Rift Browser Proof,” uses Dawnweaver with
  `rift_needle`, `dawn_barrier`, `dawn_field`, `dawn_ray`. Converting that document
  with compiled Rust produced exactly the same full JSON preset as the browser's
  separate Combat Test download.
- A freshly built current server admitted that exact downloaded level 6 preset.
  An ordinary Q cast damaged the training dummy by 33.4443 and consumed mana
  (160 to 147.13 with regeneration). This uses normal resources, server-owned
  damage, a real cast packet and authoritative analytics, not synthetic damage.
- A freshly built current client rendered the same mixed recipe with all 68
  packaged presentation profiles and matching fingerprint `02c190ee87f9aab3`.
  Rift Needle's release/impact and borrowed Q icon with Dawnweaver WER icons were
  visually inspected. One English 1280×720 native view was used. Existing capture
  tooling recorded the four-slot sequence in one run; no class/device matrix.
  That visual fixture intentionally uses level 10, infinite resource and Agnes for
  repeatability, separate from the normal-resource level 6 download proof above.
  Input was machine-driven; manual/physical-device input is not claimed.
- Independent API/shared and portal review found and fixed upstream 404/403 being
  mistaken for revoked consent, incompatible browser recovery being overwritten,
  delayed saves attaching to a replacement editor, and JSON normalization
  differences. The portal owns its final browser regression/build evidence in
  `omoba-web/docs/class-workshop.md`.
- The marketing creator section's four regression checks and production build
  passed after adding the workshop link.

Evidence is retained locally under `.agent/tasks/CLASS-WORKSHOP-20261010/` in
game and portal. Current native proof is `raw/browser-native-proof-current/`;
current renderer proof is `raw/browser-native-render-current/`. Earlier binaries
were rejected as final presentation evidence after version/config drift was
found; current-source rebuilds and reruns replaced those results.

## Prototype boundaries

The local demo is not a deployment or release certification. Public build
publishing/moderation, exact Studio requirement bindings, interactive animation
comparison and automatically approved cosmetic sets are not implemented. Existing
Studio uploads and manual contribution briefs remain available. Ekza was tested
through a visibly labelled local protocol fixture requiring explicit consent;
hosted account linking still needs operator verification before public rollout.
