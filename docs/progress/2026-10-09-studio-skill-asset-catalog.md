# Studio skill asset catalogue — planning audit

Owner request: investigate class/skill model briefs for artists, preserve the idea
in an actionable TODO and open a separate draft PR from main.

Base: `00cff781179e448c34835fa93f39e24901e7e328`; branch
`docs/studio-skill-asset-catalog`. This branch does not include Wildspark PR #77.

Read-only audit covered OMOBA canonical catalogues and presentation bindings,
Registry/Studio project/profile submissions and previews, SDK asset selection,
the player portal and marketing landing. Source revisions and concrete file
locations are recorded in the [proposal](../plans/studio-skill-asset-catalog.md).
The landing had existing local edits; no other repository was changed.

Main finding: the existing asset lifecycle should be reused. The missing piece
is the game-owned semantic graph linking skills to model requirements and a
reviewed binding from each requirement to exact contributed rendition bytes.
A single explicit handheld currently does not become two mode-specific guns.
Some effects are procedural despite a model path being present in configuration.

Deliverables: proposed versioned manifest/API resources, Wildspark inventory,
artist/reviewer route, S0–S8 tasks with owners/dependencies/acceptance, and links
from the existing creator roadmap, feature inventory and changelog. Discovery,
contribution, runtime sets and new prop capabilities have separate completion gates.

Verification: relative document/file links, example JSON syntax and references,
main-only ancestry and `git diff --check`. Documentation-only: no compilation,
new tests, migration, upload/publication, infrastructure or deployment changes.
