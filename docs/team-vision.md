# Team vision and gameplay brush

The server owns sight and sends each player only the visible battlefield state.
The client renders that state and its fog mask. Rebuild both server and client to
use this feature; an older server that omits `vision` uses the legacy presentation.

## Gameplay rules

- Living, joined allied heroes provide 32 units of sight, minions 22, towers 28,
  and base towers 34. Vision is shared across the team. A dead hero contributes
  no sight but can still see through living teammates and allied units.
- Ten mirrored, traversable circular grass patches are authored in
  `shared/src/vision.rs`. Their exact boundaries also drive the 3D grass footprint.
  These grass patches are distinct from decorative shrubs and impassable forest.
- Enemy heroes inside grass are hidden from observers outside that patch, even
  inside normal sight range. Any living allied sight source in the same patch can
  reveal them within its radius. Minions and neutral creatures use radial sight.
- Accepted basic attacks and hostile unit-targeted skills reveal the attacker for
  two seconds through grass, provided they are within the opposing team's radial
  sight. Self skills and rejected commands do not trigger this reveal.
- A local `BRUSH · CONCEALED` / `BRUSH · REVEALED` label reports the server's current
  concealment result. An already-launched homing projectile still follows and
  damages its target after concealment. New target-locked attacks require current
  visibility. Future ground-area abilities will need their own visibility rules.
- Bot, minion and tower acquisition follows the same visibility rules. The explicit
  developer Combat Test sandbox retains full visibility for measurement.

This is radial shared vision plus brush, without terrain line of sight, wards,
stealth items or remembered enemy silhouettes. The distinction between sight and
brush reveal is informed by Riot's [13.22 attack-from-fog notes](https://www.leagueoflegends.com/en-gb/news/game-updates/patch-13-22-notes/);
these are OMOBA's own timings and distances, not a claim of identical Wild Rift rules.

## Replication and presentation

Enemy heroes, minions, structures and neutrals outside sight are absent from the
recipient's live actor list. Projectile endpoints/owners, combat events, minion
target IDs and butterfly collection receipts are filtered or scrubbed as well.
A visible killing-hit receipt survives removal of a dead nonhero target, while
unseen impact locations and hidden living targets remain filtered.
Public scoreboard identity and statistics remain available. Public lobby/draft
rosters are unaffected. Hidden actors and their child visuals are despawned and
recreated normally on reacquisition; target validation clears stale selections.

Only the 3D presentation is extended. A reusable 160×96 RGBA screen fog mask and
80×80 minimap mask update at most every 75 ms, using up to 256 allied sources.
Soft boundaries span three world units; the hard authority boundary remains exact.
Grass patches outside same-patch sight are shaded. The screen mask projects onto
the ground plane: it is a lightweight mobile-oriented overlay, not volumetric fog.
The ten patches share one 33-triangle tuft mesh, one ground-disc mesh, four
materials and 310 gently swaying tuft entities. Fog/status overlays never capture
pointer input. No production dependencies or external assets were added.

## Reproducible verification

`python3 scripts/capture_team_vision.py --client-bin <client> --server-bin <server>
--assets <client/assets> --output <empty-directory>` runs a native 3D client and two
admitted protocol observers. Add `--touch-controls --width 1280 --height 720` for
the mobile control preview. This is native desktop execution, not physical iPad QA.

A development-only `OMOBA_VISION_QA=1` placement fixture starts the participants
beside the first brush. Six transitions use ordinary movement: visible outside,
hidden inside, same-brush detection, concealment restored, exit/reacquisition and
outside team sight. The driver retains both teams' actual UDP snapshots, checks
exact capture ticks, and verifies models, descendants, health bars, target state,
minimap icons and live fog. It never substitutes images or visibility state.

Raw results and the independent verdict live under
`.agent/tasks/TASK-TEAM-VISION-BRUSH-2026-09-23/` in the development checkout.
