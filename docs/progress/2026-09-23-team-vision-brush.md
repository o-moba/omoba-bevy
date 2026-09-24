# Team vision and brush — 2026-09-23

Task: TASK-TEAM-VISION-BRUSH-2026-09-23. Release: 0.23.0-rc.4.

Implemented authoritative per-team radial sight, symmetric traversable grass,
brief hostile-action reveal, recipient privacy and consistent targeted combat/AI.
The 3D client now renders live soft fog, minimap fog, swaying grass and concealment
status. No 2D presentation work was included.

Already-launched homing attacks continue after a target hides, while replication
still filters concealed positions. Terrain occlusion and wards are deferred.
Rendering uses fixed shared assets and low-resolution reused masks; no new
production dependency was introduced. See `docs/team-vision.md` for exact rules.

Verification and native desktop/mobile preview evidence are recorded in the task
proof bundle. Physical iPad performance and installation are not asserted by native
mobile-viewport capture. Both server and client must be rebuilt for this protocol.

Final checks:911 tests passed across shared/server/client and the complete UDP
harness coverage,18 existing database/integration tests ignored. Twelve final native
3D desktop/mobile-preview captures pass. Extended transport checks also caught and
fixed visible killing-hit receipts disappearing after dead nonhero targets left
snapshot arrays. Existing harness setups were migrated from omniscient assumptions
to ordinary team sight, with current shared balance values and framed transport.
The independent task verdict records the final source-bound acceptance results.
