# OMOBA dev integration — 2026-10-29

Integrate game PR136(capacity),141(compact HUD),142(iOS return) and139(browser
practice), plus concurrent PR143(RU-only OneTap API compatibility), together with portal PR20(recovery copy),22(career/browser) and25(iOS
association). Preserve all feature commit ancestry for normal PR closure. Draft
labels reflect pending rollout/device gates; source is reviewed and tested
separately. Main promotions and production/dev service deployments are outside
this integration.

Game conflicts are additive changelog/feature documentation. Portal conflicts
must retain the newer website-first registration, signed game-profile linking,
cross-tab re-review and legal notices while adding recovery copy, career polish
and return destinations. HTTP fixtures are updated to the actual lower-case
VK PKCE method and website registration reply; real providers are not contacted.

PR141 contained an internal portrait bearing but no long connector. The combined
client adds an outlined, non-picking gold line from the selected live portrait
to the current projected hero position, with clipped offscreen edge arrows.
It uses the same exact identity/visibility/life/context gates and current camera
poses; position-free public death timers cannot generate it.

Task-local proof is under `.agent/tasks/DEV-PR-BATCH-20261029/`. The visual scope
is English791×360 with only visible/offscreen selected-target states. Android
build450013 will retain the existing Beta signer; built/installed/physical tests
are distinct. iOS DNS/TLS/capabilities/profile refresh and server capacity host
measurements remain rollout gates. No server deployment, main merge or release
publication is part of this source integration.
