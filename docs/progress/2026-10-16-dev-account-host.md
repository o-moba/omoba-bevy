# Isolated DEV Account API host setup

Integrated testing required a hosted account service in addition to the lobby.
The host helper now records the setup in version control, keeps credentials
stable and private, exposes only signed native routes without the website ingress
key, and binds the runtime service to loopback. Activation verifies the immutable
DEV binary hash/revision and migration 6 before changing host state.

Verification: 11 focused security/activation regressions, source review, and a
read-only preflight on the exact dev host. Runtime rollout evidence is recorded
separately by the integrated playtest delivery. No production service or real
payment provider is changed. The runtime candidate remains 07aa4082; this patch
only adds deployment tooling and documentation.
